//! SQL the object tree's actions run or copy, made from the metadata model
//! only (never read from the database): the DDL of a node (Generate DDL to
//! Clipboard, Quick Documentation), and the `DROP`, `TRUNCATE` and rename
//! statements of the Drop, Truncate and Rename dialogs. Keywords are
//! written in lower case, names quoted only where the dialect needs it and
//! qualified with their schema (SQLite: never).
//!
//! What the model does not hold (the text of a view or a routine read
//! below level 3) becomes a comment saying so, not a guess.

use db_core::schema::{
    ConstraintKind, IdentityKind, ObjectDetail, ObjectInfo, TypeCategory, TypeDetail,
};
use db_model::{Group, NodeKind, NodeRef, SchemaModel};
use sql_syntax::Dialect;

/// The text layer's dialect of a backend's.
pub fn dialect(backend: db_core::Backend) -> Dialect {
    match backend.dialect() {
        db_core::Dialect::Oracle => Dialect::Oracle,
        db_core::Dialect::Postgres => Dialect::Postgres,
        db_core::Dialect::Sqlite => Dialect::Sqlite,
    }
}

/// A name as written in SQL.
pub fn quote(dialect: Dialect, name: &str) -> String {
    dialect.quote(name)
}

/// `schema.name` (SQLite: `name`).
fn qualify(dialect: Dialect, schema: &str, name: &str) -> String {
    match dialect {
        Dialect::Sqlite => quote(dialect, name),
        _ => format!("{}.{}", quote(dialect, schema), quote(dialect, name)),
    }
}

/// The node's qualified name: `demo.customers`; a column, key or index of
/// a table after its table: `demo.customers.email`.
pub fn reference(dialect: Dialect, node: NodeRef<'_>) -> String {
    let schema = node.schema().name();
    match node.kind() {
        NodeKind::Schema => quote(dialect, schema),
        _ => match node.parent().filter(|p| p.kind() != NodeKind::Schema) {
            Some(parent) => format!(
                "{}.{}",
                reference(dialect, parent),
                quote(dialect, node.name())
            ),
            None => qualify(dialect, schema, node.name()),
        },
    }
}

/// The qualified name of a node that is a schema object itself.
fn object_name(dialect: Dialect, node: NodeRef<'_>) -> String {
    qualify(dialect, node.schema().name(), node.name())
}

/// The table (or view) a column, key, index or trigger belongs to.
fn owner<'a>(node: NodeRef<'a>) -> Option<NodeRef<'a>> {
    node.parent().filter(|p| {
        matches!(
            p.kind(),
            NodeKind::Table | NodeKind::View | NodeKind::MaterializedView
        )
    })
}

/// The table of an index or trigger listed under its schema (its table is
/// not in the model): from the object's parent name.
fn owner_name(dialect: Dialect, node: NodeRef<'_>) -> Option<String> {
    if let Some(table) = owner(node) {
        return Some(object_name(dialect, table));
    }
    let parent = node.object()?.parent.as_deref()?;
    Some(qualify(dialect, node.schema().name(), parent))
}

/// The identity arguments of a routine: the model's signature, else the
/// types of its input arguments.
fn routine_arguments(node: NodeRef<'_>) -> String {
    if !node.signature().is_empty() {
        return node.signature().to_owned();
    }
    let Some(ObjectDetail::Routine(r)) = node.object().and_then(|o| o.detail.as_ref()) else {
        return String::new();
    };
    r.arguments
        .iter()
        .filter(|a| a.mode != db_core::schema::ArgumentMode::Out)
        .map(|a| a.data_type.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a node is called in statements and in the dialogs' texts.
pub fn kind_word(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Schema => "schema",
        NodeKind::Table => "table",
        NodeKind::View => "view",
        NodeKind::MaterializedView => "materialized view",
        NodeKind::Sequence => "sequence",
        NodeKind::Index => "index",
        NodeKind::Trigger => "trigger",
        NodeKind::Procedure => "procedure",
        NodeKind::Function => "function",
        NodeKind::Package => "package",
        NodeKind::Type => "type",
        NodeKind::Synonym => "synonym",
        NodeKind::Column => "column",
        NodeKind::PrimaryKey | NodeKind::UniqueKey => "key",
        NodeKind::ForeignKey => "foreign key",
        NodeKind::Check => "check",
        NodeKind::Argument => "argument",
    }
}

/// What a `DROP` statement looks like (the Drop dialog's options).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DropOptions {
    pub if_exists: bool,
    pub cascade: bool,
}

/// The statement that drops `node`; `None` for what cannot be dropped on
/// its own (an argument).
pub fn drop_statement(dialect: Dialect, node: NodeRef<'_>, options: DropOptions) -> Option<String> {
    let if_exists = if options.if_exists { "if exists " } else { "" };
    let cascade = match (options.cascade, dialect) {
        (false, _) | (true, Dialect::Sqlite) => "",
        (true, Dialect::Oracle) => match node.kind() {
            NodeKind::Table => " cascade constraints",
            NodeKind::Schema => " cascade",
            _ => "",
        },
        (true, _) => " cascade",
    };
    let name = || object_name(dialect, node);
    let text = match node.kind() {
        NodeKind::Schema => match dialect {
            Dialect::Oracle => format!(
                "drop user {if_exists}{}{cascade}",
                quote(dialect, node.name())
            ),
            _ => format!(
                "drop schema {if_exists}{}{cascade}",
                quote(dialect, node.name())
            ),
        },
        NodeKind::Table
        | NodeKind::View
        | NodeKind::MaterializedView
        | NodeKind::Sequence
        | NodeKind::Package
        | NodeKind::Type
        | NodeKind::Synonym => {
            format!(
                "drop {} {if_exists}{}{cascade}",
                kind_word(node.kind()),
                name()
            )
        }
        NodeKind::Index => format!("drop index {if_exists}{}{cascade}", name()),
        NodeKind::Trigger => match dialect {
            Dialect::Postgres => format!(
                "drop trigger {if_exists}{} on {}{cascade}",
                quote(dialect, node.name()),
                owner_name(dialect, node)?
            ),
            _ => format!("drop trigger {if_exists}{}", name()),
        },
        NodeKind::Function | NodeKind::Procedure => match dialect {
            Dialect::Postgres => format!(
                "drop {} {if_exists}{}({}){cascade}",
                kind_word(node.kind()),
                name(),
                routine_arguments(node)
            ),
            _ => format!("drop {} {if_exists}{}", kind_word(node.kind()), name()),
        },
        NodeKind::Column => format!(
            "alter table {} drop column {if_exists}{}{cascade}",
            object_name(dialect, owner(node)?),
            quote(dialect, node.name())
        ),
        NodeKind::PrimaryKey | NodeKind::UniqueKey | NodeKind::ForeignKey | NodeKind::Check => {
            format!(
                "alter table {} drop constraint {if_exists}{}{cascade}",
                object_name(dialect, owner(node)?),
                quote(dialect, node.name())
            )
        }
        NodeKind::Argument => return None,
    };
    Some(text + ";")
}

/// `drop database name;`
pub fn drop_database(dialect: Dialect, name: &str, options: DropOptions) -> String {
    let if_exists = if options.if_exists { "if exists " } else { "" };
    format!("drop database {if_exists}{};", quote(dialect, name))
}

/// The statement that renames `node` to `to`; `None` where the dialect
/// cannot rename it with a statement.
pub fn rename_statement(dialect: Dialect, node: NodeRef<'_>, to: &str) -> Option<String> {
    let new = quote(dialect, to);
    let name = || object_name(dialect, node);
    let text = match (node.kind(), dialect) {
        (NodeKind::Schema, Dialect::Postgres) => {
            format!(
                "alter schema {} rename to {new}",
                quote(dialect, node.name())
            )
        }
        (NodeKind::Table, _) => format!("alter table {} rename to {new}", name()),
        (NodeKind::Column, _) => format!(
            "alter table {} rename column {} to {new}",
            object_name(dialect, owner(node)?),
            quote(dialect, node.name())
        ),
        (
            NodeKind::PrimaryKey | NodeKind::UniqueKey | NodeKind::ForeignKey | NodeKind::Check,
            Dialect::Postgres | Dialect::Oracle,
        ) => format!(
            "alter table {} rename constraint {} to {new}",
            object_name(dialect, owner(node)?),
            quote(dialect, node.name())
        ),
        (NodeKind::Index, Dialect::Postgres | Dialect::Oracle) => {
            format!("alter index {} rename to {new}", name())
        }
        (NodeKind::Trigger, Dialect::Postgres) => format!(
            "alter trigger {} on {} rename to {new}",
            quote(dialect, node.name()),
            owner_name(dialect, node)?
        ),
        (NodeKind::Trigger, Dialect::Oracle) => {
            format!("alter trigger {} rename to {new}", name())
        }
        (
            NodeKind::View | NodeKind::MaterializedView | NodeKind::Sequence | NodeKind::Type,
            Dialect::Postgres,
        ) => format!(
            "alter {} {} rename to {new}",
            kind_word(node.kind()),
            name()
        ),
        (NodeKind::Function | NodeKind::Procedure, Dialect::Postgres) => format!(
            "alter {} {}({}) rename to {new}",
            kind_word(node.kind()),
            name(),
            routine_arguments(node)
        ),
        // Oracle renames views, sequences and synonyms of the current
        // schema only.
        (NodeKind::View | NodeKind::Sequence | NodeKind::Synonym, Dialect::Oracle) => {
            format!("rename {} to {new}", quote(dialect, node.name()))
        }
        _ => return None,
    };
    Some(text + ";")
}

/// `alter database name rename to new;` (PostgreSQL).
pub fn rename_database(dialect: Dialect, name: &str, to: &str) -> String {
    format!(
        "alter database {} rename to {};",
        quote(dialect, name),
        quote(dialect, to)
    )
}

/// The statement that empties a table (SQLite has no `TRUNCATE`).
pub fn truncate_statement(dialect: Dialect, node: NodeRef<'_>) -> String {
    match dialect {
        Dialect::Sqlite => format!("delete from {};", object_name(dialect, node)),
        _ => format!("truncate table {};", object_name(dialect, node)),
    }
}

/// A string literal.
fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The DDL of a node: a schema with its objects, an object with its
/// columns, keys, indexes and triggers, a column, key or index alone.
pub fn node_ddl(dialect: Dialect, node: NodeRef<'_>) -> String {
    let mut out = Vec::new();
    push_node(dialect, node, &mut out);
    join(out)
}

/// The DDL of the schemas of a data source.
pub fn schemas_ddl<'a>(dialect: Dialect, schemas: impl Iterator<Item = &'a SchemaModel>) -> String {
    let mut out = Vec::new();
    for schema in schemas {
        push_node(dialect, schema.root(), &mut out);
    }
    join(out)
}

/// The DDL of the nodes of a group.
pub fn nodes_ddl<'a>(dialect: Dialect, nodes: impl IntoIterator<Item = NodeRef<'a>>) -> String {
    let mut out = Vec::new();
    for node in nodes {
        push_node(dialect, node, &mut out);
    }
    join(out)
}

fn join(statements: Vec<String>) -> String {
    let mut text = statements.join("\n\n");
    if !text.is_empty() {
        text.push('\n');
    }
    text
}

fn push_node(dialect: Dialect, node: NodeRef<'_>, out: &mut Vec<String>) {
    match node.kind() {
        NodeKind::Schema => {
            if dialect != Dialect::Sqlite {
                out.push(format!("create schema {};", quote(dialect, node.name())));
            }
            push_comment(
                dialect,
                "schema",
                &quote(dialect, node.name()),
                node.comment(),
                out,
            );
            // Objects in the order of the tree's groups; indexes and
            // triggers come with their tables.
            for group in [
                Group::Types,
                Group::Sequences,
                Group::Tables,
                Group::Views,
                Group::MaterializedViews,
                Group::Functions,
                Group::Procedures,
                Group::Packages,
                Group::Synonyms,
                Group::Indexes,
                Group::Triggers,
            ] {
                for child in node.group(group).nodes() {
                    push_node(dialect, child, out);
                }
            }
        }
        NodeKind::Table => {
            out.push(create_table(dialect, node));
            push_comment(
                dialect,
                "table",
                &object_name(dialect, node),
                node.comment(),
                out,
            );
            for column in node.group(Group::Columns).nodes() {
                push_comment(
                    dialect,
                    "column",
                    &format!(
                        "{}.{}",
                        object_name(dialect, node),
                        quote(dialect, column.name())
                    ),
                    column.comment(),
                    out,
                );
            }
            push_indexes(dialect, node, out);
            for trigger in node.group(Group::Triggers).nodes() {
                push_node(dialect, trigger, out);
            }
            for partition in node.group(Group::Partitions).nodes() {
                push_node(dialect, partition, out);
            }
        }
        NodeKind::View | NodeKind::MaterializedView => {
            out.push(not_loaded(node));
            for index in node.group(Group::Indexes).nodes() {
                if let Some(text) = create_index(dialect, index) {
                    out.push(text);
                }
            }
            for trigger in node.group(Group::Triggers).nodes() {
                push_node(dialect, trigger, out);
            }
        }
        NodeKind::Sequence => out.push(create_sequence(dialect, node)),
        NodeKind::Type => out.push(create_type(dialect, node)),
        NodeKind::Index => match create_index(dialect, node) {
            Some(text) => out.push(text),
            None => out.push(not_loaded(node)),
        },
        NodeKind::Trigger => out.push(create_trigger(dialect, node)),
        NodeKind::Column => {
            if let Some(table) = owner(node) {
                out.push(format!(
                    "alter table {} add {};",
                    object_name(dialect, table),
                    column_definition(dialect, node)
                ));
            }
        }
        NodeKind::PrimaryKey | NodeKind::UniqueKey | NodeKind::ForeignKey | NodeKind::Check => {
            if let (Some(table), Some(text)) = (owner(node), constraint_definition(dialect, node)) {
                out.push(format!(
                    "alter table {}\n    add {text};",
                    object_name(dialect, table)
                ));
            }
        }
        NodeKind::Function | NodeKind::Procedure | NodeKind::Package | NodeKind::Synonym => {
            out.push(not_loaded(node))
        }
        NodeKind::Argument => {}
    }
}

/// The comment an object whose text is not in the model gets.
fn not_loaded(node: NodeRef<'_>) -> String {
    format!(
        "-- {} {}.{}: its definition is not in the local model",
        kind_word(node.kind()),
        node.schema().name(),
        node.name()
    )
}

fn push_comment(
    dialect: Dialect,
    what: &str,
    name: &str,
    comment: Option<&str>,
    out: &mut Vec<String>,
) {
    if dialect == Dialect::Sqlite {
        return;
    }
    if let Some(comment) = comment.filter(|c| !c.is_empty()) {
        out.push(format!("comment on {what} {name} is {};", literal(comment)));
    }
}

fn create_table(dialect: Dialect, node: NodeRef<'_>) -> String {
    let mut lines: Vec<String> = node
        .group(Group::Columns)
        .nodes()
        .map(|c| column_definition(dialect, c))
        .collect();
    for group in [Group::Keys, Group::ForeignKeys, Group::Checks] {
        for key in node.group(group).nodes() {
            if let Some(text) = constraint_definition(dialect, key) {
                lines.push(text.replace('\n', "\n    "));
            }
        }
    }
    let partition = match node.object().and_then(|o| o.detail.as_ref()) {
        Some(ObjectDetail::Table(t)) => t.partition_key.clone(),
        _ => None,
    };
    let body = if lines.is_empty() {
        "()".to_owned()
    } else {
        format!("(\n    {}\n)", lines.join(",\n    "))
    };
    let mut text = format!("create table {}\n{body}", object_name(dialect, node));
    if let Some(key) = partition {
        text.push_str(&format!("\n    partition by {key}"));
    }
    text.push(';');
    text
}

fn column_definition(dialect: Dialect, node: NodeRef<'_>) -> String {
    let Some(c) = node.column() else {
        return quote(dialect, node.name());
    };
    let mut text = format!("{} {}", quote(dialect, &c.name), c.data_type);
    if let Some(generated) = &c.generated {
        if let Some(expression) = &generated.expression {
            text.push_str(&format!(" generated always as ({expression})"));
            if generated.stored {
                text.push_str(" stored");
            }
        }
    } else if let Some(identity) = c.identity {
        text.push_str(match identity {
            IdentityKind::Always => " generated always as identity",
            IdentityKind::ByDefault => " generated by default as identity",
        });
    } else if let Some(default) = &c.default {
        text.push_str(&format!(" default {default}"));
    }
    if !c.nullable {
        text.push_str(" not null");
    }
    text
}

fn constraint_definition(dialect: Dialect, node: NodeRef<'_>) -> Option<String> {
    let c = node.constraint()?;
    let columns = |names: &[String]| {
        names
            .iter()
            .map(|n| quote(dialect, n))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let body = match c.kind {
        ConstraintKind::PrimaryKey => format!("primary key ({})", columns(&c.columns)),
        ConstraintKind::Unique => format!("unique ({})", columns(&c.columns)),
        ConstraintKind::ForeignKey => {
            let table = c.ref_table.as_deref().unwrap_or_default();
            let schema = c.ref_schema.as_deref().unwrap_or(node.schema().name());
            let mut text = format!(
                "foreign key ({}) references {} ({})",
                columns(&c.columns),
                qualify(dialect, schema, table),
                columns(&c.ref_columns)
            );
            for (what, action) in [("delete", &c.on_delete), ("update", &c.on_update)] {
                if let Some(action) = action.as_deref().filter(|a| {
                    !a.eq_ignore_ascii_case("no action") && !a.eq_ignore_ascii_case("restrict")
                }) {
                    text.push_str(&format!(" on {what} {}", action.to_lowercase()));
                }
            }
            text
        }
        ConstraintKind::Check => {
            let check = c.check.as_deref().unwrap_or("true");
            if check
                .get(..5)
                .is_some_and(|h| h.eq_ignore_ascii_case("check"))
            {
                lower_check(check)
            } else {
                format!("check ({check})")
            }
        }
    };
    Some(format!(
        "constraint {}\n    {body}",
        quote(dialect, node.name())
    ))
}

/// The indexes of a table that no key makes.
fn push_indexes(dialect: Dialect, table: NodeRef<'_>, out: &mut Vec<String>) {
    let keys: Vec<&str> = table.group(Group::Keys).nodes().map(|k| k.name()).collect();
    for index in table.group(Group::Indexes).nodes() {
        if keys.contains(&index.name()) {
            continue;
        }
        if let Some(text) = create_index(dialect, index) {
            out.push(text);
        }
    }
}

fn create_index(dialect: Dialect, node: NodeRef<'_>) -> Option<String> {
    let index = node.index()?;
    let table = owner_name(dialect, node)?;
    let unique = if index.unique { "unique " } else { "" };
    let name = match dialect {
        // PostgreSQL creates an index in its table's schema.
        Dialect::Postgres => quote(dialect, node.name()),
        _ => object_name(dialect, node),
    };
    let method = match index.method.as_deref() {
        Some(m) if dialect == Dialect::Postgres && !m.eq_ignore_ascii_case("btree") => {
            format!(" using {m}")
        }
        _ => String::new(),
    };
    let columns: Vec<String> = index
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if index.expressions.contains(&i) || c.contains('(') {
                c.clone()
            } else {
                quote(dialect, c)
            }
        })
        .collect();
    let mut text = format!(
        "create {unique}index {name}\n    on {table}{method} ({})",
        columns.join(", ")
    );
    if !index.include.is_empty() {
        let include: Vec<String> = index.include.iter().map(|c| quote(dialect, c)).collect();
        text.push_str(&format!(" include ({})", include.join(", ")));
    }
    if let Some(predicate) = &index.predicate {
        text.push_str(&format!("\n    where {predicate}"));
    }
    text.push(';');
    Some(text)
}

fn create_sequence(dialect: Dialect, node: NodeRef<'_>) -> String {
    let mut text = format!("create sequence {}", object_name(dialect, node));
    if let Some(ObjectDetail::Sequence(s)) = node.object().and_then(|o| o.detail.as_ref()) {
        if let Some(t) = s
            .data_type
            .as_deref()
            .filter(|_| dialect == Dialect::Postgres)
        {
            text.push_str(&format!("\n    as {t}"));
        }
        for (word, value) in [
            ("increment by", &s.increment),
            ("minvalue", &s.min),
            ("maxvalue", &s.max),
            ("start with", &s.start),
        ] {
            if let Some(v) = value {
                text.push_str(&format!("\n    {word} {v}"));
            }
        }
        if let Some(cache) = s.cache.filter(|c| *c > 1) {
            text.push_str(&format!("\n    cache {cache}"));
        }
        if s.cycle {
            text.push_str("\n    cycle");
        }
    }
    text.push(';');
    text
}

fn create_type(dialect: Dialect, node: NodeRef<'_>) -> String {
    let name = object_name(dialect, node);
    let Some(ObjectDetail::Type(t)) = node.object().and_then(|o| o.detail.as_ref()) else {
        return not_loaded(node);
    };
    type_text(dialect, &name, t).unwrap_or_else(|| not_loaded(node))
}

fn type_text(dialect: Dialect, name: &str, t: &TypeDetail) -> Option<String> {
    let attributes = || {
        t.attributes
            .iter()
            .map(|a| format!("    {} {}", quote(dialect, &a.name), a.data_type))
            .collect::<Vec<_>>()
            .join(",\n")
    };
    Some(match t.category {
        TypeCategory::Enum => {
            let labels: Vec<String> = t.labels.iter().map(|l| literal(l)).collect();
            format!("create type {name} as enum ({});", labels.join(", "))
        }
        TypeCategory::Composite => format!("create type {name} as\n(\n{}\n);", attributes()),
        TypeCategory::Object => format!("create type {name} as object\n(\n{}\n);", attributes()),
        TypeCategory::Domain => {
            let mut text = format!("create domain {name} as {}", t.base_type.as_deref()?);
            if let Some(d) = &t.default {
                text.push_str(&format!("\n    default {d}"));
            }
            if t.not_null {
                text.push_str("\n    not null");
            }
            for c in &t.constraints {
                text.push_str(&format!("\n    {}", lower_check(c)));
            }
            text.push(';');
            text
        }
        TypeCategory::Range => format!(
            "create type {name} as range (subtype = {});",
            t.base_type.as_deref()?
        ),
        TypeCategory::Collection | TypeCategory::Other => return None,
    })
}

/// `CHECK (...)` with its keyword in lower case.
fn lower_check(text: &str) -> String {
    match text.get(..5) {
        Some(head) if head.eq_ignore_ascii_case("check") => format!("check{}", &text[5..]),
        _ => text.to_owned(),
    }
}

fn create_trigger(dialect: Dialect, node: NodeRef<'_>) -> String {
    let info: Option<&ObjectInfo> = node.object();
    let (Some(ObjectDetail::Trigger(t)), Dialect::Postgres) =
        (info.and_then(|o| o.detail.as_ref()), dialect)
    else {
        return not_loaded(node);
    };
    let (Some(table), Some(function)) = (owner_name(dialect, node), t.function.as_deref()) else {
        return not_loaded(node);
    };
    let timing = t.timing.as_deref().unwrap_or("after").to_lowercase();
    let events = t.events.join(" or ").to_lowercase();
    let each = if t.each_row { "row" } else { "statement" };
    let mut text = format!(
        "create trigger {}\n    {timing} {events}\n    on {table}\n    for each {each}",
        quote(dialect, node.name())
    );
    if let Some(condition) = &t.condition {
        text.push_str(&format!("\n    when ({condition})"));
    }
    let call = if function.ends_with(')') {
        function.to_owned()
    } else {
        format!("{function}()")
    };
    text.push_str(&format!("\nexecute function {call};"));
    text
}

#[cfg(test)]
mod tests;
