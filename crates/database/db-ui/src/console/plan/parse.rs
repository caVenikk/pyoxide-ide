//! Readers of the answers of `EXPLAIN`: each returns the root of the plan,
//! or `None` when the answer is not the expected shape (the raw text then
//! stands alone).

use std::collections::HashMap;

use db_core::{Row, Value};
use sql_syntax::{Dialect, TokenKind};

use super::{NodeKind, PlanNode};

/// What the root of the plan is called: the statement's first word
/// (`Select`, `Insert`, ...; a `WITH` is a `Select`).
pub(super) fn statement_label(statement: &str, dialect: Dialect) -> String {
    let word = sql_syntax::lex(statement, dialect)
        .into_iter()
        .filter(|t| !t.is_trivia() && t.kind == TokenKind::Word)
        .map(|t| t.text(statement).to_lowercase())
        .next();
    match word.as_deref() {
        None => "Statement".to_owned(),
        Some("with" | "values" | "table") => "Select".to_owned(),
        Some(w) => {
            let mut chars = w.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

fn root(label: &str) -> PlanNode {
    PlanNode {
        kind: NodeKind::Statement,
        operation: label.to_owned(),
        props: vec![("Operation".into(), label.to_lowercase())],
        ..PlanNode::default()
    }
}

// ---- PostgreSQL -------------------------------------------------------------------

/// Rebuild the tree from the same text rows displayed in the Raw grid.
pub(super) fn postgres(text: &str, label: &str) -> Option<PlanNode> {
    let mut statement = root(label);
    let mut stack: Vec<(usize, PlanNode)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let arrow = trimmed.strip_prefix("->");
        let body = arrow.unwrap_or(trimmed).trim_start();
        let operation = (arrow.is_some() || stack.is_empty() && indent == 0)
            .then(|| pg_text_node(body))
            .flatten();
        if let Some(node) = operation {
            while stack.last().is_some_and(|(depth, _)| *depth >= indent) {
                finish_node(&mut stack, &mut statement);
            }
            stack.push((indent, node));
        } else if !body.is_empty() {
            if indent == 0 {
                while !stack.is_empty() {
                    finish_node(&mut stack, &mut statement);
                }
            }
            let target = stack
                .iter_mut()
                .rev()
                .find(|(depth, _)| *depth < indent)
                .map(|(_, node)| node)
                .unwrap_or(&mut statement);
            let (key, value) = body.split_once(':').unwrap_or(("Detail", body));
            target.props.push((key.into(), value.trim().into()));
            if !target.raw_desc.is_empty() {
                target.raw_desc.push_str("; ");
            }
            target.raw_desc.push_str(body);
        }
    }
    while !stack.is_empty() {
        finish_node(&mut stack, &mut statement);
    }
    (!statement.children.is_empty()).then_some(statement)
}

fn finish_node(stack: &mut Vec<(usize, PlanNode)>, root: &mut PlanNode) {
    if let Some((_, node)) = stack.pop() {
        stack
            .last_mut()
            .map(|(_, parent)| parent)
            .unwrap_or(root)
            .children
            .push(node);
    }
}

// A quoted index can itself contain the words " on ".
fn pg_split_name<'a>(name: &'a str, separator: &str) -> (&'a str, &'a str) {
    let mut quoted = false;
    for (i, ch) in name.char_indices() {
        if ch == '"' {
            quoted = !quoted;
        } else if !quoted && name[i..].starts_with(separator) {
            return (&name[..i], &name[i + separator.len()..]);
        }
    }
    (name, "")
}

fn pg_text_node(line: &str) -> Option<PlanNode> {
    let (name, metrics) = line.rsplit_once("  (cost=")?;
    let (estimated, actual) = metrics.split_once(')')?;
    let name = name.trim();
    let (operation, relation) = pg_split_name(name, " on ");
    let (operation, index) = pg_split_name(operation, " using ");
    let (kind, operation) = pg_operation(operation);
    let mut props = vec![("Operation".into(), operation.clone())];
    let mut params = Vec::new();
    if !relation.is_empty() {
        props.push(("Relation".into(), relation.into()));
        params.push(format!("table: {relation};"));
    }
    if !index.is_empty() {
        props.push(("Index".into(), index.into()));
        params.push(format!("index: {index};"));
    }
    let mut node = PlanNode {
        kind,
        operation,
        params: params.join(" "),
        props,
        ..Default::default()
    };
    if actual.trim() == "(never executed)" {
        node.props
            .push(("Execution".into(), "never executed".into()));
        node.raw_desc = "never executed".into();
    }
    let (cost, estimates) = estimated.split_once(' ')?;
    (node.startup_cost, node.total_cost) = pg_range(cost);
    node.props.push(("Cost".into(), cost.into()));
    for (key, value) in estimates
        .split_whitespace()
        .filter_map(|field| field.split_once('='))
    {
        if key == "rows" {
            node.rows = value.parse().ok();
        }
        node.props.push((key.into(), value.into()));
    }
    if let Some(metrics) = actual
        .trim()
        .strip_prefix("(actual ")
        .and_then(|s| s.strip_suffix(')'))
    {
        for (key, value) in metrics
            .split_whitespace()
            .filter_map(|field| field.split_once('='))
        {
            match key {
                "time" => (node.actual_startup_time, node.actual_total_time) = pg_range(value),
                "rows" => node.actual_rows = value.parse().ok(),
                _ => {}
            }
            node.props.push((format!("actual {key}"), value.into()));
        }
    }
    Some(node)
}

fn pg_range(value: &str) -> (Option<f64>, Option<f64>) {
    value
        .split_once("..")
        .map(|(a, b)| (a.parse().ok(), b.parse().ok()))
        .unwrap_or_default()
}

/// The name of an operation as the table shows it, and its kind.
fn pg_operation(node_type: &str) -> (NodeKind, String) {
    match node_type {
        "Seq Scan" => (NodeKind::Scan, "Full Scan (Seq Scan)".into()),
        "Hash" => (NodeKind::Transform, "Transformation (Hash)".into()),
        "Sort" | "Incremental Sort" | "Materialize" | "Memoize" => {
            (NodeKind::Transform, node_type.into())
        }
        t if t.starts_with("Index") => (NodeKind::IndexScan, format!("Index Scan ({t})")),
        t if t.starts_with("Bitmap") => (NodeKind::IndexScan, format!("Bitmap Scan ({t})")),
        t if t.ends_with("Join") || t == "Nested Loop" => (NodeKind::Join, t.into()),
        t if t.ends_with("Scan") => (NodeKind::Scan, format!("Scan ({t})")),
        t => (NodeKind::Other, t.into()),
    }
}

// ---- SQLite -----------------------------------------------------------------------

/// The rows `id, parent, notused, detail` of `EXPLAIN QUERY PLAN`: parents
/// by id; the ones without a known parent are the root's children.
pub(super) fn sqlite(rows: &[Row], label: &str) -> Option<PlanNode> {
    struct Entry {
        id: i64,
        parent: i64,
        detail: String,
    }
    let entries: Option<Vec<Entry>> = rows
        .iter()
        .map(|r| {
            let (Some(Value::Int(id)), Some(Value::Int(parent)), Some(Value::Text(detail))) =
                (r.first(), r.get(1), r.get(3))
            else {
                return None;
            };
            Some(Entry {
                id: *id,
                parent: *parent,
                detail: detail.clone(),
            })
        })
        .collect();
    let entries = entries?;
    let mut by_parent: HashMap<i64, Vec<usize>> = HashMap::new();
    for (ix, e) in entries.iter().enumerate() {
        let known = entries.iter().any(|p| p.id == e.parent);
        by_parent
            .entry(if known { e.parent } else { 0 })
            .or_default()
            .push(ix);
    }
    fn build(
        ix: usize,
        entries: &[Entry],
        by_parent: &HashMap<i64, Vec<usize>>,
        depth: usize,
    ) -> PlanNode {
        let e = &entries[ix];
        let (operation, params) = match e.detail.split_once(' ') {
            Some((verb @ ("SCAN" | "SEARCH"), rest)) => (verb.to_owned(), rest.to_owned()),
            _ => (e.detail.clone(), String::new()),
        };
        let kind = match operation.as_str() {
            "SEARCH" => NodeKind::IndexScan,
            "SCAN" if params.contains(" USING ") => NodeKind::IndexScan,
            "SCAN" => NodeKind::Scan,
            o if o.starts_with("USE TEMP B-TREE") => NodeKind::Transform,
            _ => NodeKind::Other,
        };
        let children = match depth < 64 {
            true => by_parent
                .get(&e.id)
                .map(|c| {
                    c.iter()
                        .filter(|&&c| c != ix)
                        .map(|&c| build(c, entries, by_parent, depth + 1))
                        .collect()
                })
                .unwrap_or_default(),
            false => Vec::new(),
        };
        PlanNode {
            kind,
            props: vec![
                ("Operation".into(), operation.clone()),
                ("Detail".into(), e.detail.clone()),
                ("Id".into(), e.id.to_string()),
                ("Parent".into(), e.parent.to_string()),
            ],
            operation,
            params,
            children,
            ..PlanNode::default()
        }
    }
    let mut node = root(label);
    node.children = by_parent
        .get(&0)
        .map(|c| {
            c.iter()
                .map(|&c| build(c, &entries, &by_parent, 0))
                .collect()
        })
        .unwrap_or_default();
    Some(node)
}

/// The tree as the `sqlite3` shell prints it.
pub(super) fn sqlite_text(root: &PlanNode) -> String {
    fn lines(nodes: &[PlanNode], prefix: &str, out: &mut Vec<String>) {
        for (ix, n) in nodes.iter().enumerate() {
            let last = ix + 1 == nodes.len();
            let detail = n
                .props
                .iter()
                .find(|(k, _)| k == "Detail")
                .map_or("", |(_, v)| v.as_str());
            out.push(format!(
                "{prefix}{}--{detail}",
                if last { '`' } else { '|' }
            ));
            let next = format!("{prefix}{}  ", if last { ' ' } else { '|' });
            lines(&n.children, &next, out);
        }
    }
    let mut out = vec!["QUERY PLAN".to_owned()];
    lines(&root.children, "", &mut out);
    out.join("\n")
}

// ---- Oracle -----------------------------------------------------------------------

/// The table of `DBMS_XPLAN.DISPLAY`: the operation's indentation is its
/// depth; `Id 0` (the statement) is the root. The predicate lines under
/// the table are added to their operations.
pub(super) fn oracle(text: &str) -> Option<PlanNode> {
    let lines: Vec<&str> = text.lines().collect();
    let header_at = lines
        .iter()
        .position(|l| l.starts_with('|') && l.contains(" Id ") && l.contains("Operation"))?;
    let names: Vec<String> = cells(lines[header_at])
        .iter()
        .map(|c| c.trim().to_owned())
        .collect();
    let column = |n: &str| names.iter().position(|c| c == n);
    let (id_col, op_col) = (column("Id")?, column("Operation")?);
    let (name_col, rows_col, bytes_col, cost_col, time_col) = (
        column("Name"),
        column("Rows"),
        column("Bytes"),
        names.iter().position(|c| c.starts_with("Cost")),
        column("Time"),
    );
    // (depth, id, node)
    let mut flat: Vec<(usize, String, PlanNode)> = Vec::new();
    let mut end = header_at + 1;
    for (at, line) in lines.iter().enumerate().skip(header_at + 1) {
        end = at;
        if line.starts_with('-') {
            if flat.is_empty() {
                continue;
            }
            break;
        }
        if !line.starts_with('|') {
            break;
        }
        let c = cells(line);
        let cell = |ix: Option<usize>| ix.and_then(|i| c.get(i)).map_or("", |s| s.trim());
        let raw_op = c.get(op_col)?;
        let depth = raw_op.len() - raw_op.trim_start().len();
        let id = cell(Some(id_col)).trim_start_matches('*').trim().to_owned();
        let operation = raw_op.trim().to_owned();
        let name = cell(name_col);
        let mut props = vec![("Operation".to_owned(), operation.clone())];
        for (title, ix) in [
            ("Id", Some(id_col)),
            ("Name", name_col),
            ("Rows", rows_col),
            ("Bytes", bytes_col),
            ("Cost (%CPU)", cost_col),
            ("Time", time_col),
        ] {
            let v = cell(ix);
            if !v.is_empty() {
                props.push((title.to_owned(), v.to_owned()));
            }
        }
        let node = PlanNode {
            kind: oracle_kind(&operation),
            operation,
            params: if name.is_empty() {
                String::new()
            } else {
                format!("object: {name};")
            },
            rows: oracle_number(cell(rows_col)),
            total_cost: oracle_number(cell(cost_col)),
            props,
            ..PlanNode::default()
        };
        flat.push((depth.saturating_sub(1), id, node));
    }
    if flat.is_empty() {
        return None;
    }
    // Predicates: `   1 - filter("A"=1)`, continued on the lines after.
    let mut current: Option<usize> = None;
    for line in lines
        .iter()
        .skip(end)
        .skip_while(|l| !l.starts_with("Predicate"))
    {
        let trimmed = line.trim();
        if let Some((id, rest)) = trimmed.split_once(" - ")
            && id.chars().all(|c| c.is_ascii_digit())
            && !id.is_empty()
        {
            current = flat.iter().position(|(_, i, _)| i == id);
            if let Some(ix) = current {
                flat[ix]
                    .2
                    .props
                    .push(("Predicate".to_owned(), rest.to_owned()));
            }
        } else if let Some(ix) = current
            && !trimmed.is_empty()
            && let Some((_, last)) = flat[ix].2.props.last_mut()
        {
            last.push(' ');
            last.push_str(trimmed);
        }
    }
    for (_, _, node) in &mut flat {
        node.raw_desc = node
            .props
            .iter()
            .filter(|(k, _)| k == "Predicate")
            .map(|(_, v)| format!("Predicate = {v}"))
            .collect::<Vec<_>>()
            .join("; ");
    }
    // Nest by depth with a stack of the open ancestors.
    let mut stack: Vec<(usize, PlanNode)> = Vec::new();
    for (depth, _, node) in flat {
        while let Some((d, _)) = stack.last()
            && *d >= depth
        {
            let (_, done) = stack.pop()?;
            match stack.last_mut() {
                Some((_, parent)) => parent.children.push(done),
                None => return None,
            }
        }
        stack.push((depth, node));
    }
    while stack.len() > 1 {
        let (_, done) = stack.pop()?;
        stack.last_mut()?.1.children.push(done);
    }
    let (_, mut top) = stack.pop()?;
    top.kind = NodeKind::Statement;
    Some(top)
}

/// The cells of a `| a | b |` line, with their padding.
fn cells(line: &str) -> Vec<&str> {
    let inner = line.strip_prefix('|').unwrap_or(line);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').collect()
}

fn oracle_kind(operation: &str) -> NodeKind {
    let op = operation.to_uppercase();
    if op.contains("INDEX") {
        NodeKind::IndexScan
    } else if op.contains("TABLE ACCESS") {
        NodeKind::Scan
    } else if op.contains("JOIN") || op.contains("NESTED LOOPS") {
        NodeKind::Join
    } else if op.starts_with("SORT") || op.starts_with("HASH") || op.contains("BUFFER") {
        NodeKind::Transform
    } else {
        NodeKind::Other
    }
}

/// `1`, `12K`, `3M`, `2   (0)`: the leading number, with Oracle's
/// thousand-based suffixes.
fn oracle_number(cell: &str) -> Option<f64> {
    let cell = cell.trim();
    let digits: String = cell
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let n: f64 = digits.parse().ok()?;
    let rest = cell[digits.len()..].trim_start();
    let scale = match rest.chars().next() {
        Some('K') => 1e3,
        Some('M') => 1e6,
        Some('G') => 1e9,
        Some('T') => 1e12,
        Some('P') => 1e15,
        _ => 1.0,
    };
    Some(n * scale)
}
