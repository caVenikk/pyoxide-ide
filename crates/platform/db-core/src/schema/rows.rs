use crate::{Error, Result, Row, Value};

use super::{opt_text, text};

/// One row of a metadata query with checked access: a row narrower than the
/// query's select list (a driver that drops a column, a changed catalog)
/// gives an [`Error`] naming the query and the column instead of a panic.
#[derive(Debug, Clone)]
pub(crate) struct Cells {
    query: &'static str,
    cells: Row,
}

impl Cells {
    pub(crate) fn new(query: &'static str, cells: Row) -> Self {
        Self { query, cells }
    }

    /// The cell at `index` for columns that only some variants of a query
    /// select; `None` when the row is shorter.
    pub(crate) fn get(&self, index: usize) -> Option<&Value> {
        self.cells.get(index)
    }

    /// The cell at `index`.
    pub(crate) fn value(&self, index: usize) -> Result<&Value> {
        self.cells.get(index).ok_or_else(|| {
            Error::internal(format!(
                "metadata query `{}` returned a row of {} columns, column {index} is missing",
                self.query,
                self.cells.len()
            ))
        })
    }

    /// Text of the cell; empty for NULL.
    pub(crate) fn text(&self, index: usize) -> Result<String> {
        self.value(index).map(text)
    }

    /// Text of the cell; `None` for NULL or empty.
    pub(crate) fn opt_text(&self, index: usize) -> Result<Option<String>> {
        self.value(index).map(opt_text)
    }

    /// The cell as an integer; `None` for NULL or a non-integer.
    pub(crate) fn int(&self, index: usize) -> Result<Option<i64>> {
        self.value(index).map(Value::as_i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;

    #[test]
    fn short_row_names_the_query_and_the_column() {
        let row = Cells::new("relations", vec![Value::from("t"), Value::Int(7)]);
        assert_eq!(row.text(0).unwrap(), "t");
        assert_eq!(row.int(1).unwrap(), Some(7));
        let err = row.opt_text(2).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Internal);
        assert!(err.message.contains("`relations`"), "{}", err.message);
        assert!(err.message.contains("column 2"), "{}", err.message);
        assert!(row.value(5).is_err());
        assert!(row.int(2).is_err());
    }
}
