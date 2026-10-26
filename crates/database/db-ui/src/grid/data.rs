//! What a grid holds, without drawing: the columns with their key marks,
//! the rows of the shown page as cell texts (made off the UI thread when a
//! page arrives), the selection, the sort order read from the ORDER BY
//! text, and the aggregate of the selection for the status bar.

use std::sync::Arc;

use db_core::{Column, Row, ValueKind};
use gpui::SharedString;

use super::format::{self, Decimal};

/// How a column takes part in the table's keys (header icon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyMark {
    #[default]
    None,
    Primary,
    Foreign,
}

/// A column of the grid.
#[derive(Debug, Clone, PartialEq)]
pub struct GridColumn {
    pub name: SharedString,
    /// The type as the table's metadata gives it, else the driver's name.
    pub type_name: SharedString,
    pub kind: ValueKind,
    pub right: bool,
    pub key: KeyMark,
    /// The width the content asks for, in characters of the editor font
    /// (header included), capped.
    pub chars: usize,
}

/// One cell's text; `None` for NULL.
pub type CellText = Option<SharedString>;

/// The page a grid shows.
#[derive(Debug, Clone, Default)]
pub struct GridPage {
    pub columns: Vec<GridColumn>,
    pub cells: Vec<Vec<CellText>>,
    /// The values, for the aggregates.
    pub rows: Arc<Vec<Row>>,
    pub first_row: u64,
    pub has_more: bool,
}

/// Longest content a column widens for, in characters.
pub const MAX_COLUMN_CHARS: usize = 50;
/// Narrowest column, in characters.
const MIN_COLUMN_CHARS: usize = 4;

/// What the table's metadata says about a column (by name).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColumnMeta {
    pub name: String,
    pub data_type: String,
    pub key: KeyMark,
}

impl GridPage {
    /// Builds the cell texts of a page (the slow part: run it off the UI
    /// thread).
    pub fn build(
        columns: &[Column],
        rows: Vec<Row>,
        first_row: u64,
        has_more: bool,
        meta: &[ColumnMeta],
    ) -> Self {
        let mut grid_columns: Vec<GridColumn> = columns
            .iter()
            .map(|c| {
                let meta = meta.iter().find(|m| m.name == c.name);
                let type_name = meta
                    .map(|m| m.data_type.clone())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| c.type_name.clone());
                GridColumn {
                    name: c.name.clone().into(),
                    type_name: type_name.into(),
                    kind: c.kind,
                    right: format::right_aligned(c.kind),
                    key: meta.map(|m| m.key).unwrap_or_default(),
                    // The header's room is added when the width is laid out.
                    chars: 0,
                }
            })
            .collect();
        let cells: Vec<Vec<CellText>> = rows
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let text = format::cell_text(v);
                        if let Some(c) = grid_columns.get_mut(i) {
                            let len = text
                                .as_deref()
                                .map_or(format::NULL_TEXT.len(), |t| first_line_chars(t));
                            c.chars = c.chars.max(len.min(MAX_COLUMN_CHARS));
                        }
                        text.map(SharedString::from)
                    })
                    .collect()
            })
            .collect();
        for c in &mut grid_columns {
            c.chars = c.chars.clamp(MIN_COLUMN_CHARS, MAX_COLUMN_CHARS);
        }
        Self {
            columns: grid_columns,
            cells,
            rows: Arc::new(rows),
            first_row,
            has_more,
        }
    }

    pub fn row_count(&self) -> usize {
        self.cells.len()
    }

    pub fn column_count(&self) -> usize {
        self.columns.len()
    }

    pub fn cell(&self, row: usize, column: usize) -> Option<&CellText> {
        self.cells.get(row)?.get(column)
    }
}

fn first_line_chars(text: &str) -> usize {
    text.lines().next().unwrap_or("").chars().count()
}

/// A cell: row and column in the shown page.
pub type Pos = (usize, usize);

/// The selected cells: a rectangle from the anchor (where it started) to
/// the lead (the current cell), as the reference's grid selects with the
/// mouse and Shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    pub anchor: Pos,
    pub lead: Pos,
}

impl Selection {
    pub fn cell(pos: Pos) -> Self {
        Self {
            anchor: pos,
            lead: pos,
        }
    }

    pub fn rows(&self) -> std::ops::RangeInclusive<usize> {
        self.anchor.0.min(self.lead.0)..=self.anchor.0.max(self.lead.0)
    }

    pub fn columns(&self) -> std::ops::RangeInclusive<usize> {
        self.anchor.1.min(self.lead.1)..=self.anchor.1.max(self.lead.1)
    }

    pub fn contains(&self, (row, column): Pos) -> bool {
        self.rows().contains(&row) && self.columns().contains(&column)
    }

    pub fn cell_count(&self) -> usize {
        self.rows().count() * self.columns().count()
    }

    /// Fits the selection into a page of `rows` x `columns`.
    pub fn clamp(self, rows: usize, columns: usize) -> Option<Self> {
        if rows == 0 || columns == 0 {
            return None;
        }
        let fit = |(r, c): Pos| (r.min(rows - 1), c.min(columns - 1));
        Some(Self {
            anchor: fit(self.anchor),
            lead: fit(self.lead),
        })
    }
}

/// The status bar's text for a selection: the sum of its numbers, the cell
/// and row counts when more than one cell is selected, and where the
/// current cell is (`SUM: 0   500 cells, 500 rows   1:3`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionSummary {
    pub sum: String,
    /// `500 cells, 500 rows`; `None` for one cell.
    pub counts: Option<String>,
    /// `row:column`, one-based in the page.
    pub position: String,
}

pub fn summary(page: &GridPage, selection: &Selection) -> SelectionSummary {
    let mut sum = Decimal::default();
    let mut overflow = false;
    for r in selection.rows() {
        let Some(row) = page.rows.get(r) else {
            continue;
        };
        for c in selection.columns() {
            if let Some(n) = row.get(c).and_then(Decimal::of) {
                match sum.checked_add(n) {
                    Some(s) => sum = s,
                    None => overflow = true,
                }
            }
        }
    }
    let cells = selection.cell_count();
    let rows = selection.rows().count();
    SelectionSummary {
        sum: if overflow {
            "\u{221e}".to_owned()
        } else {
            sum.to_string()
        },
        counts: (cells > 1).then(|| {
            format!(
                "{cells} {}, {rows} {}",
                if cells == 1 { "cell" } else { "cells" },
                if rows == 1 { "row" } else { "rows" }
            )
        }),
        position: format!("{}:{}", selection.lead.0 + 1, selection.lead.1 + 1),
    }
}

/// The direction a column is sorted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

/// Which columns the ORDER BY text sorts by, when it is a plain list of
/// column names with an optional direction each (`at desc, id`); anything
/// else (expressions, positions) marks no header.
pub fn sort_of(order: &str, columns: &[GridColumn]) -> Vec<(usize, SortDir)> {
    let mut out = Vec::new();
    for item in order.split(',') {
        let mut words = item.split_whitespace();
        let Some(name) = words.next() else {
            continue;
        };
        let dir = match words.next().map(str::to_ascii_lowercase).as_deref() {
            None | Some("asc") => SortDir::Asc,
            Some("desc") => SortDir::Desc,
            _ => return Vec::new(),
        };
        if words
            .next()
            .is_some_and(|w| !w.eq_ignore_ascii_case("nulls"))
        {
            return Vec::new();
        }
        let unquoted = name.trim_matches('"');
        let Some(ix) = columns.iter().position(|c| {
            if name.starts_with('"') {
                c.name == unquoted
            } else {
                c.name.eq_ignore_ascii_case(unquoted)
            }
        }) else {
            return Vec::new();
        };
        out.push((ix, dir));
    }
    out
}

/// The ORDER BY text after a click on a column's sort mark: ascending,
/// then descending, then unsorted, replacing any other order.
pub fn next_order(order: &str, columns: &[GridColumn], column: usize, quoted: String) -> String {
    let current = sort_of(order, columns);
    match current.as_slice() {
        [(c, SortDir::Asc)] if *c == column => format!("{quoted} desc"),
        [(c, SortDir::Desc)] if *c == column => String::new(),
        _ => quoted,
    }
}

#[cfg(test)]
mod tests {
    use db_core::Value;

    use super::*;

    fn columns() -> Vec<Column> {
        vec![
            Column {
                name: "id".into(),
                type_name: "int8".into(),
                kind: ValueKind::Int,
                nullable: false,
            },
            Column {
                name: "kind".into(),
                type_name: "text".into(),
                kind: ValueKind::Text,
                nullable: true,
            },
        ]
    }

    fn page() -> GridPage {
        let rows = vec![
            vec![Value::Int(1), Value::Text("view".into())],
            vec![Value::Int(2), Value::Null],
            vec![Value::Int(39), Value::Text("a long text".into())],
        ];
        let meta = [ColumnMeta {
            name: "id".into(),
            data_type: "bigint".into(),
            key: KeyMark::Primary,
        }];
        GridPage::build(&columns(), rows, 0, true, &meta)
    }

    #[test]
    fn a_page_has_cell_texts_widths_and_keys() {
        let p = page();
        assert_eq!(p.row_count(), 3);
        assert_eq!(p.columns[0].key, KeyMark::Primary);
        assert_eq!(p.columns[0].type_name, "bigint");
        assert_eq!(p.columns[1].type_name, "text");
        assert!(p.columns[0].right);
        assert_eq!(p.cell(1, 1), Some(&None));
        assert_eq!(
            p.cell(0, 1).unwrap().as_ref().map(|t| t.as_ref()),
            Some("view")
        );
        assert_eq!(p.columns[1].chars, "a long text".len());
        assert_eq!(p.columns[0].chars, MIN_COLUMN_CHARS);
    }

    #[test]
    fn the_summary_sums_numbers_of_the_selection() {
        let p = page();
        let all = Selection {
            anchor: (0, 0),
            lead: (2, 1),
        };
        let s = summary(&p, &all);
        assert_eq!(s.sum, "42");
        assert_eq!(s.counts.as_deref(), Some("6 cells, 3 rows"));
        assert_eq!(s.position, "3:2");
        let one = summary(&p, &Selection::cell((0, 1)));
        assert_eq!(one.sum, "0");
        assert_eq!(one.counts, None);
        assert_eq!(one.position, "1:2");
    }

    #[test]
    fn sort_marks_follow_the_order_text() {
        let p = page();
        let c = &p.columns;
        assert_eq!(sort_of("id", c), vec![(0, SortDir::Asc)]);
        assert_eq!(
            sort_of("KIND desc, id", c),
            vec![(1, SortDir::Desc), (0, SortDir::Asc)]
        );
        assert_eq!(sort_of("id desc nulls last", c), vec![(0, SortDir::Desc)]);
        assert!(sort_of("lower(kind)", c).is_empty());
        assert!(sort_of("", c).is_empty());
        assert_eq!(next_order("", c, 0, "id".into()), "id");
        assert_eq!(next_order("id", c, 0, "id".into()), "id desc");
        assert_eq!(next_order("id desc", c, 0, "id".into()), "");
        assert_eq!(next_order("id desc", c, 1, "kind".into()), "kind");
    }

    #[test]
    fn selections_fit_the_page() {
        let s = Selection {
            anchor: (9, 9),
            lead: (1, 0),
        };
        assert_eq!(s.rows(), 1..=9);
        assert_eq!(s.cell_count(), 90);
        let fit = s.clamp(3, 2).unwrap();
        assert_eq!(fit.anchor, (2, 1));
        assert!(s.clamp(0, 2).is_none());
        assert!(fit.contains((2, 0)));
    }
}
