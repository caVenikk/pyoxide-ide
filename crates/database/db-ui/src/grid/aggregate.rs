//! The aggregates of the selected cells (the value panel's Aggregates tab,
//! scenario grid-copy-and-views): the figures the reference lists in
//! alphabetical order. Numbers are summed and compared exactly (no
//! rounding to a float); an average and a median that do not divide
//! evenly have the reference's 34 significant digits.

use super::data::{GridPage, Selection};
use super::format::Decimal;

/// What the tab shows when a figure needs more numbers than are selected.
pub const NOT_ENOUGH: &str = "Not enough values";

/// The figures of a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregates {
    pub rows: usize,
    pub cols: usize,
    /// Every selected cell, NULLs included.
    pub count: usize,
    /// The cells that are numbers.
    pub count_nums: usize,
    /// `0` without numbers; `∞` when the numbers outgrow the exact range.
    pub sum: String,
    pub min: Option<String>,
    pub max: Option<String>,
    pub avg: Option<String>,
    pub median: Option<String>,
    /// The population standard deviation over the mean, in per cent (the
    /// reference's 57.73% for the 438 numbers of the demo `events.ratio`).
    pub variation: Option<String>,
}

impl Aggregates {
    /// The rows of the tab: the reference's names in alphabetical order.
    pub fn lines(&self) -> Vec<(&'static str, String)> {
        let figure = |v: &Option<String>| v.clone().unwrap_or_else(|| NOT_ENOUGH.to_owned());
        vec![
            ("AVG", figure(&self.avg)),
            ("COEFFICIENT_VARIATION", figure(&self.variation)),
            ("COLS", self.cols.to_string()),
            ("COUNT", self.count.to_string()),
            ("COUNT_NUMS", self.count_nums.to_string()),
            ("MAX", figure(&self.max)),
            ("MEDIAN", figure(&self.median)),
            ("MIN", figure(&self.min)),
            ("ROWS", self.rows.to_string()),
            ("SUM", self.sum.clone()),
        ]
    }

    /// The figure `name` (`SUM`, `AVG`, ...) as text.
    pub fn get(&self, name: &str) -> Option<String> {
        self.lines()
            .into_iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }
}

/// The figures of `selection` over the columns `shown` lets through.
pub fn aggregates(
    page: &GridPage,
    selection: &Selection,
    shown: &dyn Fn(usize) -> bool,
) -> Aggregates {
    let columns: Vec<usize> = selection.columns().filter(|c| shown(*c)).collect();
    let rows = selection.rows().filter(|r| *r < page.rows.len()).count();
    let mut numbers: Vec<Decimal> = Vec::new();
    for r in selection.rows() {
        let Some(row) = page.rows.get(r) else {
            continue;
        };
        for &c in &columns {
            if let Some(n) = row.get(c).and_then(Decimal::of) {
                numbers.push(n);
            }
        }
    }
    let mut sum = Decimal::default();
    let mut overflow = false;
    for n in &numbers {
        match sum.checked_add(*n) {
            Some(s) => sum = s,
            None => overflow = true,
        }
    }
    let count = numbers.len();
    let sorted = {
        let mut v = numbers.clone();
        v.sort_by(|a, b| a.compare(*b).unwrap_or(std::cmp::Ordering::Equal));
        v
    };
    let (min, max) = (sorted.first().copied(), sorted.last().copied());
    let median = match sorted.len() {
        0 => None,
        n if n % 2 == 1 => Some(sorted[n / 2].to_string()),
        n => sorted[n / 2 - 1]
            .checked_add(sorted[n / 2])
            .map(|s| s.divided_by(2)),
    };
    let avg = (count > 0 && !overflow).then(|| sum.divided_by(count as u64));
    let variation = (count > 1)
        .then(|| {
            let values: Vec<f64> = numbers.iter().map(|n| n.to_f64()).collect();
            let mean = values.iter().sum::<f64>() / count as f64;
            let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count as f64;
            (mean != 0.0 && var.is_finite())
                .then(|| format!("{:.2}%", var.sqrt() / mean.abs() * 100.0))
        })
        .flatten();
    Aggregates {
        rows,
        cols: columns.len(),
        count: rows * columns.len(),
        count_nums: count,
        sum: if overflow {
            "\u{221e}".to_owned()
        } else {
            sum.to_string()
        },
        min: min.map(|m| m.to_string()),
        max: max.map(|m| m.to_string()),
        avg,
        median,
        variation,
    }
}

#[cfg(test)]
mod tests {
    use db_core::{Column, Value, ValueKind};

    use super::*;

    fn page(rows: Vec<Vec<Value>>) -> GridPage {
        let columns = |n: usize| -> Vec<Column> {
            (0..n)
                .map(|i| Column {
                    name: format!("c{i}"),
                    type_name: "x".into(),
                    kind: ValueKind::Other,
                    nullable: true,
                })
                .collect()
        };
        let n = rows.first().map_or(0, Vec::len);
        GridPage::build(&columns(n), rows, 0, false, &[], &Default::default())
    }

    fn all(p: &GridPage) -> Selection {
        Selection {
            anchor: (0, 0),
            lead: (p.row_count() - 1, p.column_count() - 1),
        }
    }

    #[test]
    fn the_figures_of_numbers_with_nulls_and_text() {
        let p = page(vec![
            vec![Value::Decimal("0.001".into()), Value::Text("a".into())],
            vec![Value::Decimal("0.002".into()), Value::Null],
            vec![Value::Null, Value::Text("b".into())],
            vec![Value::Float(0.25), Value::Int(1)],
        ]);
        let a = aggregates(&p, &all(&p), &|_| true);
        assert_eq!(a.rows, 4);
        assert_eq!(a.cols, 2);
        assert_eq!(a.count, 8);
        assert_eq!(a.count_nums, 4);
        assert_eq!(a.sum, "1.253");
        assert_eq!(a.min.as_deref(), Some("0.001"));
        assert_eq!(a.max.as_deref(), Some("1"));
        // Four numbers: the middle two are 0.002 and 0.25.
        assert_eq!(a.median.as_deref(), Some("0.126"));
        assert_eq!(a.avg.as_deref(), Some("0.31325"));
        assert!(a.variation.as_deref().unwrap().ends_with('%'));
        let lines = a.lines();
        assert_eq!(lines[0], ("AVG", "0.31325".to_owned()));
        assert_eq!(lines[9], ("SUM", "1.253".to_owned()));
        assert_eq!(a.get("sum").as_deref(), Some("1.253"));
    }

    #[test]
    fn a_cell_without_a_number_has_nothing_to_average() {
        let p = page(vec![vec![Value::Text("x".into())]]);
        let a = aggregates(&p, &all(&p), &|_| true);
        assert_eq!((a.count, a.count_nums, a.rows, a.cols), (1, 0, 1, 1));
        assert_eq!(a.sum, "0");
        let lines = a.lines();
        assert_eq!(lines[0].1, NOT_ENOUGH);
        assert_eq!(lines[1].1, NOT_ENOUGH);
        assert_eq!(lines[5].1, NOT_ENOUGH);
        assert_eq!(lines[6].1, NOT_ENOUGH);
        assert_eq!(lines[7].1, NOT_ENOUGH);
        // One number: an average and the extremes, but no spread.
        let p = page(vec![vec![Value::Int(7)]]);
        let a = aggregates(&p, &all(&p), &|_| true);
        assert_eq!(a.avg.as_deref(), Some("7"));
        assert_eq!(a.median.as_deref(), Some("7"));
        assert_eq!(a.variation, None);
    }

    #[test]
    fn hidden_columns_do_not_count() {
        let p = page(vec![vec![Value::Int(1), Value::Int(10)]; 3]);
        let a = aggregates(&p, &all(&p), &|c| c == 0);
        assert_eq!((a.cols, a.count, a.sum.as_str()), (1, 3, "3"));
    }

    #[test]
    fn exact_beyond_sixty_four_bits() {
        let p = page(vec![
            vec![Value::Decimal("123456789012345678901234567890".into())],
            vec![Value::Decimal("1".into())],
        ]);
        let a = aggregates(&p, &all(&p), &|_| true);
        assert_eq!(a.sum, "123456789012345678901234567891");
    }
}
