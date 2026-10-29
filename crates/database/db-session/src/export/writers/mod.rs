//! The format writers: a result goes through one [`RowWriter`] a row at a
//! time, so a result of any size is written with the memory of one row
//! (only the transposed layout holds the whole result).

mod json;
mod literal;
mod sql;
mod tabular;

use std::io::{self, Write};

use db_core::{Column, Value};

pub use literal::{quote as sql_quote, sql_condition, sql_literal, value_text};

use super::format::{ExportOptions, Format};

/// Writes a result in one format: [`RowWriter::begin`] once, then every row,
/// then [`RowWriter::finish`].
pub trait RowWriter: Send {
    fn begin(&mut self, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }

    fn row(&mut self, out: &mut dyn Write, row: &[Value]) -> io::Result<()>;

    fn finish(&mut self, _out: &mut dyn Write) -> io::Result<()> {
        Ok(())
    }
}

/// The writer of `options.format` for a result with `columns`.
pub fn writer(options: &ExportOptions, columns: &[Column]) -> Box<dyn RowWriter> {
    match options.format {
        Format::Json => Box::new(json::JsonWriter::new(columns)),
        Format::SqlInserts => Box::new(sql::Inserts::new(options, columns)),
        Format::SqlUpdates => Box::new(sql::Updates::new(options, columns)),
        Format::WhereClause => Box::new(sql::Where::new(options, columns)),
        _ => Box::new(tabular::TextWriter::new(options, columns)),
    }
}

/// Writes `rows` in `options.format` to a string (copying a selection, the
/// export preview). Rows are taken as they are.
pub fn to_string(options: &ExportOptions, columns: &[Column], rows: &[Vec<Value>]) -> String {
    let mut out = Vec::new();
    let mut w = writer(options, columns);
    // A write to memory cannot fail.
    let _ = w.begin(&mut out);
    for row in rows {
        let _ = w.row(&mut out, row);
    }
    let _ = w.finish(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests;
