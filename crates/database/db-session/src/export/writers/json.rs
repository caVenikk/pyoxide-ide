//! JSON: an array with one object per row, values typed (numbers, booleans,
//! null, JSON documents embedded, anything else as text).

use std::io::{self, Write};

use db_core::{Column, Value};

use super::RowWriter;
use super::literal::value_text;

pub(super) struct JsonWriter {
    names: Vec<String>,
    rows: u64,
}

impl JsonWriter {
    pub(super) fn new(columns: &[Column]) -> Self {
        Self {
            names: columns.iter().map(|c| c.name.clone()).collect(),
            rows: 0,
        }
    }
}

fn string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

/// Whether `text` is a JSON number as it stands.
fn is_json_number(text: &str) -> bool {
    text.trim() == text
        && !text.is_empty()
        && matches!(
            serde_json::from_str::<serde_json::Value>(text),
            Ok(serde_json::Value::Number(_))
        )
}

pub(super) fn json_value(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) if f.is_finite() => serde_json::Number::from_f64(*f)
            .map(|n| n.to_string())
            .unwrap_or_else(|| string(&f.to_string())),
        Value::Decimal(s) if is_json_number(s) => s.clone(),
        Value::Json(s) if s.trim() == s && serde_json::from_str::<serde_json::Value>(s).is_ok() => {
            s.clone()
        }
        Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(json_value).collect();
            format!("[{}]", inner.join(", "))
        }
        Value::Truncated { prefix, .. } => json_value(prefix),
        other => string(&value_text(other)),
    }
}

impl RowWriter for JsonWriter {
    fn begin(&mut self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(b"[")
    }

    fn row(&mut self, out: &mut dyn Write, row: &[Value]) -> io::Result<()> {
        out.write_all(if self.rows == 0 { b"\n" } else { b",\n" })?;
        self.rows += 1;
        out.write_all(b"  {")?;
        for (i, (name, v)) in self.names.iter().zip(row).enumerate() {
            if i > 0 {
                out.write_all(b", ")?;
            }
            write!(out, "{}: {}", string(name), json_value(v))?;
        }
        out.write_all(b"}")
    }

    fn finish(&mut self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(if self.rows == 0 { b"]\n" } else { b"\n]\n" })
    }
}
