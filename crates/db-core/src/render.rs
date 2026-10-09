use crate::{Cell, ResultSet};

/// Render a result set as a plain-text table (for the CLI and logs).
pub fn render_table(rs: &ResultSet) -> String {
    if rs.columns.is_empty() {
        return format!("OK, {} row(s) affected", rs.rows_affected);
    }
    let text = |c: &Cell| match c {
        Cell::Null => "NULL".to_string(),
        Cell::Text(s) => s.replace('\n', "\\n"),
    };
    let mut widths: Vec<usize> = rs.columns.iter().map(|c| c.name.chars().count()).collect();
    for row in &rs.rows {
        for (i, c) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(text(c).chars().count());
            }
        }
    }
    let line = |cells: Vec<String>| {
        cells
            .iter()
            .zip(&widths)
            .map(|(s, w)| format!("{s:<w$}", w = *w))
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let mut out = String::new();
    out.push_str(&line(rs.columns.iter().map(|c| c.name.clone()).collect()));
    out.push('\n');
    out.push_str(
        &widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("-+-"),
    );
    out.push('\n');
    for row in &rs.rows {
        out.push_str(&line(row.iter().map(text).collect()));
        out.push('\n');
    }
    out.push_str(&format!(
        "({} row(s){})",
        rs.rows.len(),
        if rs.truncated { ", truncated" } else { "" }
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Column;

    fn col(n: &str) -> Column {
        Column {
            name: n.into(),
            type_name: "VARCHAR2".into(),
            nullable: true,
        }
    }

    #[test]
    fn renders_table_with_null_and_truncation() {
        let rs = ResultSet {
            columns: vec![col("ID"), col("NAME")],
            rows: vec![
                vec![Cell::Text("1".into()), Cell::Text("alice".into())],
                vec![Cell::Text("22".into()), Cell::Null],
            ],
            rows_affected: 0,
            truncated: true,
        };
        let s = render_table(&rs);
        assert!(s.contains("ID | NAME"));
        assert!(s.contains("22 | NULL"));
        assert!(s.ends_with("(2 row(s), truncated)"));
    }

    #[test]
    fn renders_dml() {
        let rs = ResultSet {
            rows_affected: 3,
            ..Default::default()
        };
        assert_eq!(render_table(&rs), "OK, 3 row(s) affected");
    }
}
