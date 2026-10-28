//! The grid's other views (the toolbar's View As: Table, Transpose, Text;
//! scenario grid-copy-and-views).
//!
//! Transpose turns the page on its side: a row for each column of the
//! table, a column for each row of the page, the first column holding the
//! column names. It is a [`GridPage`] of its own, so selection, scrolling
//! and the panel work on it as on any page. Text shows the page as comma
//! separated text in a read-only editor, the reference's text view of the
//! CSV extractor: no header line, a field with a comma, quote or line
//! break in quotes with the quotes doubled, NULL as nothing.

use std::sync::Arc;

use db_core::{Value, ValueKind};
use gpui::SharedString;

use super::data::{GridColumn, GridPage, KeyMark};

/// How the page is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Table,
    Transpose,
    Text,
}

impl ViewMode {
    pub fn id(self) -> &'static str {
        match self {
            ViewMode::Table => "table",
            ViewMode::Transpose => "transpose",
            ViewMode::Text => "text",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [ViewMode::Table, ViewMode::Transpose, ViewMode::Text]
            .into_iter()
            .find(|v| v.id() == id)
    }
}

/// The page on its side: the first column holds the column names, each
/// other column is a row of the page, headed by its number in the result.
pub fn transposed(page: &GridPage) -> GridPage {
    let rows = page.row_count();
    let mut columns = vec![GridColumn {
        name: SharedString::from(""),
        type_name: SharedString::from(""),
        kind: ValueKind::Text,
        right: false,
        key: KeyMark::None,
        chars: 0,
    }];
    for r in 0..rows {
        columns.push(GridColumn {
            name: (page.first_row + r as u64 + 1).to_string().into(),
            type_name: SharedString::from(""),
            kind: ValueKind::Other,
            right: false,
            key: KeyMark::None,
            chars: 0,
        });
    }
    let mut cells = Vec::with_capacity(page.column_count());
    let mut values = Vec::with_capacity(page.column_count());
    for (c, column) in page.columns.iter().enumerate() {
        let mut texts = vec![Some(column.name.clone())];
        let mut row = vec![Value::Text(column.name.to_string())];
        for r in 0..rows {
            texts.push(page.cell(r, c).cloned().flatten());
            row.push(page.rows[r].get(c).cloned().unwrap_or(Value::Null));
        }
        for (i, t) in texts.iter().enumerate() {
            let len = t
                .as_deref()
                .map_or(6, |t| t.lines().next().map_or(0, |l| l.chars().count()));
            columns[i].chars = columns[i].chars.max(len.min(super::data::MAX_COLUMN_CHARS));
        }
        cells.push(texts);
        values.push(row);
    }
    for c in &mut columns {
        c.chars = c.chars.clamp(4, super::data::MAX_COLUMN_CHARS);
    }
    GridPage {
        columns,
        cells,
        rows: Arc::new(values),
        first_row: 0,
        has_more: false,
    }
}

/// One CSV field: in quotes (doubled inside) when it holds a comma, a quote
/// or a line break.
pub fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// The page as CSV lines of the columns `shown` lets through.
pub fn csv_text(page: &GridPage, shown: &dyn Fn(usize) -> bool) -> String {
    let mut out = String::new();
    for row in &page.cells {
        let fields: Vec<String> = row
            .iter()
            .enumerate()
            .filter(|(c, _)| shown(*c))
            .map(|(_, t)| {
                t.as_ref()
                    .map(|t| csv_field(t.as_ref()))
                    .unwrap_or_default()
            })
            .collect();
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use db_core::{Column, Value};

    use super::*;
    use crate::grid::data::ColumnMeta;

    fn page() -> GridPage {
        let columns: Vec<Column> = ["id", "note"]
            .iter()
            .map(|n| Column {
                name: (*n).into(),
                type_name: "x".into(),
                kind: ValueKind::Text,
                nullable: true,
            })
            .collect();
        let rows = vec![
            vec![Value::Int(1), Value::Text("a, b".into())],
            vec![Value::Int(2), Value::Null],
            vec![Value::Int(3), Value::Text("say \"hi\"\nbye".into())],
        ];
        let meta: [ColumnMeta; 0] = [];
        GridPage::build(&columns, rows, 40, true, &meta, &Default::default())
    }

    #[test]
    fn text_is_csv_without_a_header() {
        let p = page();
        assert_eq!(
            csv_text(&p, &|_| true),
            "1,\"a, b\"\n2,\n3,\"say \"\"hi\"\"\nbye\"\n"
        );
        assert_eq!(csv_text(&p, &|c| c == 0), "1\n2\n3\n");
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("ünï,cödé"), "\"ünï,cödé\"");
    }

    #[test]
    fn a_transposed_page_has_a_row_per_column() {
        let t = transposed(&page());
        assert_eq!(t.column_count(), 4);
        assert_eq!(t.row_count(), 2);
        let names: Vec<&str> = t.columns.iter().map(|c| c.name.as_ref()).collect();
        // The headers are the rows' numbers in the result.
        assert_eq!(names, ["", "41", "42", "43"]);
        assert_eq!(
            t.cell(0, 0).unwrap().as_ref().map(|s| s.as_ref()),
            Some("id")
        );
        assert_eq!(
            t.cell(0, 2).unwrap().as_ref().map(|s| s.as_ref()),
            Some("2")
        );
        assert_eq!(t.cell(1, 2), Some(&None));
        assert_eq!(t.rows[1][3], Value::Text("say \"hi\"\nbye".into()));
        // Its rows are numbered from 1.
        assert_eq!(t.first_row, 0);
        assert_eq!(ViewMode::from_id("transpose"), Some(ViewMode::Transpose));
    }
}
