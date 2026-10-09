use crate::{ResultSet, Value};

/// Longest cell text the table shows before cutting it.
const MAX_CELL: usize = 80;

/// Render a result set as a plain-text table (for the CLI and logs).
pub fn render_table(rs: &ResultSet) -> String {
    if rs.columns.is_empty() {
        return format!("OK, {} row(s) affected", rs.rows_affected);
    }
    let text = |v: &Value| {
        let s = v.to_string().replace('\n', "\\n");
        if s.chars().count() > MAX_CELL {
            let cut: String = s.chars().take(MAX_CELL - 1).collect();
            format!("{cut}…")
        } else {
            s
        }
    };
    let mut widths: Vec<usize> = rs.columns.iter().map(|c| c.name.chars().count()).collect();
    for row in &rs.rows {
        for (i, v) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(text(v).chars().count());
            }
        }
    }
    let numeric: Vec<bool> = rs.columns.iter().map(|c| c.kind.is_numeric()).collect();
    let line = |cells: Vec<String>, align: bool| {
        cells
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(i, (s, w))| {
                if align && numeric.get(i).copied().unwrap_or(false) {
                    format!("{s:>w$}", w = *w)
                } else {
                    format!("{s:<w$}", w = *w)
                }
            })
            .collect::<Vec<_>>()
            .join(" | ")
            .trim_end()
            .to_owned()
    };
    let mut out = String::new();
    out.push_str(&line(
        rs.columns.iter().map(|c| c.name.clone()).collect(),
        false,
    ));
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
        out.push_str(&line(row.iter().map(text).collect(), true));
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
    use crate::{Column, ValueKind};

    fn col(n: &str, kind: ValueKind) -> Column {
        Column {
            name: n.into(),
            type_name: "X".into(),
            kind,
            nullable: true,
        }
    }

    #[test]
    fn renders_table_with_null_and_truncation() {
        let rs = ResultSet {
            columns: vec![col("ID", ValueKind::Int), col("NAME", ValueKind::Text)],
            rows: vec![
                vec![Value::Int(1), Value::from("alice")],
                vec![Value::Int(22), Value::Null],
            ],
            rows_affected: 0,
            truncated: true,
        };
        let s = render_table(&rs);
        assert!(s.contains("ID | NAME"), "{s}");
        assert!(s.contains(" 1 | alice"), "{s}");
        assert!(s.contains("22 | NULL"), "{s}");
        assert!(s.ends_with("(2 row(s), truncated)"));
    }

    #[test]
    fn cuts_long_cells() {
        let rs = ResultSet {
            columns: vec![col("T", ValueKind::Text)],
            rows: vec![vec![Value::Text("x".repeat(200))]],
            ..Default::default()
        };
        let s = render_table(&rs);
        assert!(s.contains('…'));
        assert!(s.lines().nth(2).unwrap().chars().count() <= MAX_CELL);
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
