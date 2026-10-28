//! `INSERT` and `UPDATE` statements and `WHERE` conditions of rows.

use std::io::{self, Write};

use db_core::{Column, Dialect, Value};

use super::RowWriter;
use super::literal::{sql_condition, sql_literal};
use crate::export::format::ExportOptions;
use crate::job::syntax_dialect;

/// What an unnamed source's statements say instead of a table.
const NO_TABLE: &str = "table_name";

fn names(columns: &[Column], dialect: Dialect) -> Vec<String> {
    let d = syntax_dialect(dialect);
    columns.iter().map(|c| d.quote(&c.name)).collect()
}

fn table(options: &ExportOptions) -> String {
    options.table.clone().unwrap_or_else(|| NO_TABLE.into())
}

pub(super) struct Inserts {
    table: String,
    columns: String,
    dialect: Dialect,
}

impl Inserts {
    pub(super) fn new(options: &ExportOptions, columns: &[Column]) -> Self {
        Self {
            table: table(options),
            columns: names(columns, options.dialect).join(", "),
            dialect: options.dialect,
        }
    }
}

impl RowWriter for Inserts {
    fn row(&mut self, out: &mut dyn Write, row: &[Value]) -> io::Result<()> {
        let values: Vec<String> = row.iter().map(|v| sql_literal(v, self.dialect)).collect();
        writeln!(
            out,
            "INSERT INTO {} ({}) VALUES ({});",
            self.table,
            self.columns,
            values.join(", ")
        )
    }
}

pub(super) struct Updates {
    table: String,
    names: Vec<String>,
    /// Indexes of the columns that identify a row.
    keys: Vec<usize>,
    dialect: Dialect,
}

impl Updates {
    pub(super) fn new(options: &ExportOptions, columns: &[Column]) -> Self {
        let mut keys: Vec<usize> = columns
            .iter()
            .enumerate()
            .filter(|(_, c)| options.key_columns.iter().any(|k| *k == c.name))
            .map(|(i, _)| i)
            .collect();
        if keys.is_empty() {
            keys = (0..columns.len()).collect();
        }
        Self {
            table: table(options),
            names: names(columns, options.dialect),
            keys,
            dialect: options.dialect,
        }
    }
}

impl RowWriter for Updates {
    fn row(&mut self, out: &mut dyn Write, row: &[Value]) -> io::Result<()> {
        let mut set: Vec<String> = (0..self.names.len())
            .filter(|i| !self.keys.contains(i))
            .filter_map(|i| {
                Some(format!(
                    "{} = {}",
                    self.names[i],
                    sql_literal(row.get(i)?, self.dialect)
                ))
            })
            .collect();
        // Every column is a key: nothing else to set, the row is set to
        // itself.
        if set.is_empty() {
            set = self
                .keys
                .iter()
                .filter_map(|&i| {
                    Some(format!(
                        "{} = {}",
                        self.names[i],
                        sql_literal(row.get(i)?, self.dialect)
                    ))
                })
                .collect();
        }
        let conditions: Vec<String> = self
            .keys
            .iter()
            .filter_map(|&i| Some(sql_condition(&self.names[i], row.get(i)?, self.dialect)))
            .collect();
        writeln!(
            out,
            "UPDATE {} SET {} WHERE {};",
            self.table,
            set.join(", "),
            conditions.join(" AND ")
        )
    }
}

/// A condition that matches the rows: `(a = 1 AND b = 'x')` for each,
/// joined by `OR`.
pub(super) struct Where {
    names: Vec<String>,
    dialect: Dialect,
    rows: u64,
}

impl Where {
    pub(super) fn new(options: &ExportOptions, columns: &[Column]) -> Self {
        Self {
            names: names(columns, options.dialect),
            dialect: options.dialect,
            rows: 0,
        }
    }
}

impl RowWriter for Where {
    fn row(&mut self, out: &mut dyn Write, row: &[Value]) -> io::Result<()> {
        if self.rows > 0 {
            out.write_all(b"\nOR ")?;
        }
        self.rows += 1;
        let parts: Vec<String> = self
            .names
            .iter()
            .zip(row)
            .map(|(n, v)| sql_condition(n, v, self.dialect))
            .collect();
        if self.names.len() > 1 {
            write!(out, "({})", parts.join(" AND "))
        } else {
            write!(out, "{}", parts.join(" AND "))
        }
    }
}
