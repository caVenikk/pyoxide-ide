//! Quick documentation texts (Markdown) of the objects of the metadata
//! model, of built-in functions, data types and keywords.

use std::collections::HashMap;

use db_core::schema::{ArgumentMode, ObjectDetail};
use db_model::{Group, Model, NodeId, NodeKind, NodeRef};
use sql_syntax::Dialect;

use crate::builtins::{DataType, FnKind, Function, keyword_doc};

/// How many columns a table's documentation lists.
const MAX_COLUMNS: usize = 60;

fn fence(line: &str) -> String {
    format!("```sql\n{line}\n```\n")
}

fn qualified(dialect: Dialect, n: NodeRef<'_>) -> String {
    match n.kind() {
        NodeKind::Schema => dialect.quote(n.name()),
        _ => format!(
            "{}.{}",
            dialect.quote(n.schema().name()),
            dialect.quote(n.name())
        ),
    }
}

/// The documentation of a node of the model.
pub(crate) fn node_doc(model: &Model, id: NodeId, dialect: Dialect) -> Option<String> {
    let n = model.node(id)?;
    Some(match n.kind() {
        NodeKind::Schema => schema_doc(n, dialect),
        k if k.is_relation() => relation_doc(n, dialect),
        NodeKind::Synonym => format!(
            "{}{}",
            fence(&format!("synonym {}", qualified(dialect, n))),
            comment(n)
        ),
        NodeKind::Column => column_doc(n, dialect),
        k if k.is_routine() => routine_doc(n, dialect),
        k => format!(
            "{}{}",
            fence(&format!(
                "{} {}",
                format!("{k:?}").to_lowercase(),
                qualified(dialect, n)
            )),
            comment(n)
        ),
    })
}

fn comment(n: NodeRef<'_>) -> String {
    match n.comment().map(str::trim).filter(|c| !c.is_empty()) {
        Some(c) => format!("\n{c}\n"),
        None => String::new(),
    }
}

fn schema_doc(n: NodeRef<'_>, dialect: Dialect) -> String {
    let mut out = fence(&format!("schema {}", qualified(dialect, n)));
    out.push_str(&comment(n));
    let count = |g: Group| n.group(g).len();
    out.push_str(&format!(
        "\n- {} tables, {} views\n- {} functions, {} procedures\n",
        count(Group::Tables),
        count(Group::Views) + count(Group::MaterializedViews),
        count(Group::Functions),
        count(Group::Procedures),
    ));
    out
}

fn relation_doc(n: NodeRef<'_>, dialect: Dialect) -> String {
    let word = match n.kind() {
        NodeKind::View => "view",
        NodeKind::MaterializedView => "materialized view",
        _ => "table",
    };
    let mut out = fence(&format!("{word} {}", qualified(dialect, n)));
    out.push_str(&comment(n));
    let mut primary: Vec<String> = Vec::new();
    let mut foreign: HashMap<String, String> = HashMap::new();
    for key in n.group(Group::Keys).nodes() {
        if key.kind() == NodeKind::PrimaryKey
            && let Some(c) = key.constraint()
        {
            primary.extend(c.columns.iter().cloned());
        }
    }
    for fk in n.group(Group::ForeignKeys).nodes() {
        if let Some(c) = fk.constraint() {
            let table = c.ref_table.clone().unwrap_or_default();
            for (col, target) in c.columns.iter().zip(&c.ref_columns) {
                foreign.insert(col.clone(), format!("{table}.{target}"));
            }
        }
    }
    let columns = n.group(Group::Columns);
    if !columns.is_loaded() {
        out.push_str("\nThe columns are not loaded yet.\n");
        return out;
    }
    if columns.is_empty() {
        return out;
    }
    out.push('\n');
    for (i, c) in columns.nodes().enumerate() {
        if i == MAX_COLUMNS {
            out.push_str(&format!("- and {} more\n", columns.len() - MAX_COLUMNS));
            break;
        }
        let Some(info) = c.column() else {
            continue;
        };
        let mut marks: Vec<String> = Vec::new();
        if primary.iter().any(|p| p == c.name()) {
            marks.push("primary key".into());
        }
        if let Some(target) = foreign.get(c.name()) {
            marks.push(format!("references {target}"));
        }
        if !info.nullable {
            marks.push("not null".into());
        }
        let tail = if marks.is_empty() {
            String::new()
        } else {
            format!(", {}", marks.join(", "))
        };
        out.push_str(&format!("- `{}` {}{tail}\n", c.name(), info.data_type));
    }
    out
}

fn column_doc(n: NodeRef<'_>, dialect: Dialect) -> String {
    let parent = n.parent();
    let table = parent.map(|p| qualified(dialect, p)).unwrap_or_default();
    let Some(info) = n.column() else {
        return String::new();
    };
    let mut out = fence(&format!(
        "column {table}.{} {}",
        dialect.quote(n.name()),
        info.data_type
    ));
    out.push_str(&comment(n));
    let mut lines: Vec<String> = Vec::new();
    lines.push(if info.nullable {
        "nullable".into()
    } else {
        "not null".into()
    });
    if let Some(d) = &info.default {
        lines.push(format!("default `{d}`"));
    }
    if info.auto_increment {
        lines.push("generated automatically".into());
    }
    if let Some(p) = parent {
        for key in p.group(Group::Keys).nodes() {
            if key.kind() == NodeKind::PrimaryKey
                && key
                    .constraint()
                    .is_some_and(|c| c.columns.iter().any(|x| x == n.name()))
            {
                lines.push("primary key".into());
            }
            if key.kind() == NodeKind::UniqueKey
                && key
                    .constraint()
                    .is_some_and(|c| c.columns.len() == 1 && c.columns[0] == n.name())
            {
                lines.push("unique".into());
            }
        }
        for fk in p.group(Group::ForeignKeys).nodes() {
            let Some(c) = fk.constraint() else {
                continue;
            };
            for (col, target) in c.columns.iter().zip(&c.ref_columns) {
                if col == n.name() {
                    lines.push(format!(
                        "references `{}.{target}`",
                        c.ref_table.clone().unwrap_or_default()
                    ));
                }
            }
        }
    }
    out.push('\n');
    for l in lines {
        out.push_str(&format!("- {l}\n"));
    }
    out
}

/// `name(arg type, ...)` of a routine of the model.
pub(crate) fn routine_signature(n: NodeRef<'_>, dialect: Dialect) -> String {
    let args: Vec<String> = n
        .group(Group::Arguments)
        .nodes()
        .filter_map(|a| a.argument().map(|i| (a, i)))
        .filter(|(_, i)| i.mode != ArgumentMode::Table)
        .map(|(a, i)| {
            let mode = match i.mode {
                ArgumentMode::Out => "out ",
                ArgumentMode::InOut => "inout ",
                ArgumentMode::Variadic => "variadic ",
                _ => "",
            };
            let name = if a.name().starts_with('$') {
                String::new()
            } else {
                format!("{} ", a.name())
            };
            format!("{mode}{name}{}", i.data_type)
        })
        .collect();
    let ret = match n.object().and_then(|o| o.detail.as_ref()) {
        Some(ObjectDetail::Routine(r)) => r.return_type.clone(),
        _ => None,
    };
    let mut s = format!("{}({})", qualified(dialect, n), args.join(", "));
    if let Some(r) = ret {
        s.push_str(&format!(" returns {r}"));
    }
    s
}

fn routine_doc(n: NodeRef<'_>, dialect: Dialect) -> String {
    let word = if n.kind() == NodeKind::Procedure {
        "procedure"
    } else {
        "function"
    };
    let mut out = fence(&format!("{word} {}", routine_signature(n, dialect)));
    out.push_str(&comment(n));
    if let Some(ObjectDetail::Routine(r)) = n.object().and_then(|o| o.detail.as_ref()) {
        let mut lines = Vec::new();
        if let Some(l) = &r.language {
            lines.push(format!("language {l}"));
        }
        if !r.characteristics.is_empty() {
            lines.push(r.characteristics.join(", ").to_lowercase());
        }
        if !lines.is_empty() {
            out.push('\n');
            for l in lines {
                out.push_str(&format!("- {l}\n"));
            }
        }
    }
    out
}

/// The documentation of a built-in function.
pub(crate) fn function_doc(f: &Function, overloads: &[&Function]) -> String {
    let mut out = fence(&f.signature());
    for o in overloads.iter().filter(|o| o.params != f.params) {
        out.push_str(&fence(&o.signature()));
    }
    out.push('\n');
    out.push_str(f.doc);
    out.push('\n');
    if f.kind == FnKind::Aggregate {
        out.push_str("\nAn aggregate function.\n");
    } else if f.kind == FnKind::Window {
        out.push_str("\nA window function: it needs an OVER clause.\n");
    }
    out
}

pub(crate) fn type_doc(t: &DataType) -> String {
    format!("{}\n{}\n", fence(&format!("{}{}", t.name, t.args)), t.doc)
}

pub(crate) fn keyword_text(word: &str) -> Option<String> {
    let doc = keyword_doc(word)?;
    Some(format!("{}\n{doc}\n", fence(&word.to_uppercase())))
}

/// What a table alias stands for.
pub(crate) fn alias_doc(
    alias: &str,
    table: Option<String>,
    columns: &[(String, Option<String>)],
) -> String {
    let mut out = match &table {
        Some(t) => fence(&format!("alias {alias} for {t}")),
        None => fence(&format!("alias {alias}")),
    };
    if !columns.is_empty() {
        out.push('\n');
        for (name, ty) in columns.iter().take(MAX_COLUMNS) {
            match ty {
                Some(t) => out.push_str(&format!("- `{name}` {t}\n")),
                None => out.push_str(&format!("- `{name}`\n")),
            }
        }
    }
    out
}

pub(crate) fn cte_doc(name: &str, columns: &[String]) -> String {
    let mut out = fence(&format!("with {name}"));
    if !columns.is_empty() {
        out.push('\n');
        for c in columns.iter().take(MAX_COLUMNS) {
            out.push_str(&format!("- `{c}`\n"));
        }
    }
    out
}
