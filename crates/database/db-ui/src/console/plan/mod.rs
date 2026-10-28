//! The plan of a statement (scenario explain): what `EXPLAIN` answered,
//! as a tree of operations with their estimates and, after an analyze, the
//! measured rows and times, next to the raw text of the answer. The
//! statements that ask come from `sql_syntax::explain_statements`; the
//! parsers read their results (PostgreSQL `EXPLAIN (FORMAT JSON)`, SQLite
//! `EXPLAIN QUERY PLAN`, Oracle `DBMS_XPLAN.DISPLAY`), the view draws the
//! tree and the properties of the selected node.

mod commands;
mod parse;
pub(crate) mod view;

pub(super) use commands::init;
pub(crate) use commands::{ExplainAnalyze, ExplainPlan};

use std::collections::HashSet;

use db_core::{Row, Value};
use sql_syntax::Dialect;

/// What a node is, for its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum NodeKind {
    /// The root: the statement itself.
    Statement,
    /// A read of a whole table.
    Scan,
    /// A read through an index.
    IndexScan,
    Join,
    /// Sorting, hashing, materializing and the like.
    Transform,
    #[default]
    Other,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PlanNode {
    pub kind: NodeKind,
    pub operation: String,
    /// The object the operation works on: `table: orders;`.
    pub params: String,
    /// Estimated rows.
    pub rows: Option<f64>,
    pub actual_rows: Option<f64>,
    pub total_cost: Option<f64>,
    pub startup_cost: Option<f64>,
    /// Milliseconds.
    pub actual_total_time: Option<f64>,
    pub actual_startup_time: Option<f64>,
    /// What the server said of the node besides the columns above.
    pub raw_desc: String,
    /// Every property of the node, in the server's order (the property
    /// table of the selected node).
    pub props: Vec<(String, String)>,
    pub children: Vec<PlanNode>,
}

/// A column of the plan table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Col {
    Operation,
    Params,
    Rows,
    ActualRows,
    TotalCost,
    ActualTotalTime,
    StartupCost,
    ActualStartupTime,
    RawDesc,
}

impl Col {
    pub fn title(self) -> &'static str {
        match self {
            Col::Operation => "Operation",
            Col::Params => "Params",
            Col::Rows => "Rows",
            Col::ActualRows => "Actual Rows",
            Col::TotalCost => "Total Cost",
            Col::ActualTotalTime => "Actual Total Time",
            Col::StartupCost => "Startup Cost",
            Col::ActualStartupTime => "Actual Startup Time",
            Col::RawDesc => "Raw Desc",
        }
    }

    /// Numbers are right-aligned.
    pub fn numeric(self) -> bool {
        !matches!(self, Col::Operation | Col::Params | Col::RawDesc)
    }

    /// Pixels the column starts with.
    pub fn width(self) -> f32 {
        match self {
            Col::Operation => 240.0,
            Col::Params => 240.0,
            Col::Rows | Col::StartupCost => 96.0,
            Col::TotalCost => 100.0,
            Col::ActualRows => 110.0,
            Col::ActualTotalTime | Col::ActualStartupTime => 150.0,
            Col::RawDesc => 220.0,
        }
    }
}

/// A row of the plan table: a node and where it is.
pub(crate) struct Line<'a> {
    /// The indexes of the children that lead to the node.
    pub path: Vec<usize>,
    pub node: &'a PlanNode,
}

pub(crate) struct Plan {
    pub root: PlanNode,
    /// The answer as the server gave it, as text.
    pub raw: String,
    /// The statement ran and the plan has its measured rows and times.
    pub analyze: bool,
}

impl Plan {
    /// Reads the rows `EXPLAIN` answered with. An answer that is not the
    /// expected shape gives a plan of the statement alone with the raw text.
    pub fn from_rows(dialect: Dialect, statement: &str, rows: &[Row], analyze: bool) -> Self {
        let label = parse::statement_label(statement, dialect);
        let text = rows_text(rows);
        let parsed = match dialect {
            Dialect::Postgres => parse::postgres(&text, &label),
            Dialect::Sqlite => parse::sqlite(rows, &label),
            Dialect::Oracle => parse::oracle(&text),
            Dialect::Generic => None,
        };
        let raw = match (dialect, &parsed) {
            (Dialect::Postgres, _) => parse::pretty_json(&text),
            (Dialect::Sqlite, Some(root)) => parse::sqlite_text(root),
            _ => text,
        };
        let root = parsed.unwrap_or_else(|| PlanNode {
            kind: NodeKind::Statement,
            operation: label.clone(),
            props: vec![("Operation".into(), label.to_lowercase())],
            ..PlanNode::default()
        });
        Self { root, raw, analyze }
    }

    /// The columns that have a value somewhere in the plan.
    pub fn columns(&self) -> Vec<Col> {
        let mut nodes = Vec::new();
        collect(&self.root, &mut nodes);
        let any = |f: fn(&PlanNode) -> bool| nodes.iter().any(|n| f(n));
        let mut cols = vec![Col::Operation, Col::Params];
        let wanted = [
            (Col::Rows, any(|n| n.rows.is_some())),
            (Col::ActualRows, any(|n| n.actual_rows.is_some())),
            (Col::TotalCost, any(|n| n.total_cost.is_some())),
            (Col::ActualTotalTime, any(|n| n.actual_total_time.is_some())),
            (Col::StartupCost, any(|n| n.startup_cost.is_some())),
            (
                Col::ActualStartupTime,
                any(|n| n.actual_startup_time.is_some()),
            ),
            (Col::RawDesc, any(|n| !n.raw_desc.is_empty())),
        ];
        cols.extend(wanted.into_iter().filter(|(_, has)| *has).map(|(c, _)| c));
        cols
    }

    /// The rows of the table, parents before children, without the
    /// descendants of a collapsed node.
    pub fn lines(&self, collapsed: &HashSet<Vec<usize>>) -> Vec<Line<'_>> {
        let mut out = Vec::new();
        walk(&self.root, Vec::new(), collapsed, &mut out);
        out
    }

    pub fn node(&self, path: &[usize]) -> Option<&PlanNode> {
        let mut node = &self.root;
        for &ix in path {
            node = node.children.get(ix)?;
        }
        Some(node)
    }
}

fn collect<'a>(node: &'a PlanNode, out: &mut Vec<&'a PlanNode>) {
    out.push(node);
    for child in &node.children {
        collect(child, out);
    }
}

fn walk<'a>(
    node: &'a PlanNode,
    path: Vec<usize>,
    collapsed: &HashSet<Vec<usize>>,
    out: &mut Vec<Line<'a>>,
) {
    let open = !collapsed.contains(&path);
    out.push(Line {
        path: path.clone(),
        node,
    });
    if !open {
        return;
    }
    for (ix, child) in node.children.iter().enumerate() {
        let mut p = path.clone();
        p.push(ix);
        walk(child, p, collapsed, out);
    }
}

/// The text of what the statements answered: the first column of every
/// row, one line each.
fn rows_text(rows: &[Row]) -> String {
    rows.iter()
        .filter_map(|r| r.first())
        .map(|v| match v {
            Value::Text(s) | Value::Json(s) => s.clone(),
            Value::Null => String::new(),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A number as the plan table shows it: no trailing zeros.
pub(crate) fn number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

#[cfg(test)]
mod tests;
