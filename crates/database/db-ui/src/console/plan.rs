//! What an Execute action runs, from the text, the caret, the selection and
//! the options of the action (Settings > Tools > Database > Query
//! Execution).

use std::ops::Range;

use sql_syntax::{CandidateKind, Dialect, Statement};

use crate::query_settings::{CaretInside, CaretOutside, ExecuteOptions, ForSelection};

/// One answer to "what to execute".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    pub label: String,
    pub statements: Vec<Statement>,
}

/// The outcome of the options applied to the editor's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Nothing to run (no statements, or the options say so).
    Nothing,
    Run(Vec<Statement>),
    /// More than one thing could run and the options ask: the person picks.
    Ask(Vec<Choice>),
}

/// The longest label of a choice, in characters.
const LABEL_CHARS: usize = 70;

/// `caret` and `selection` are byte offsets into `text`.
pub(crate) fn plan(
    text: &str,
    caret: usize,
    selection: Range<usize>,
    dialect: Dialect,
    options: &ExecuteOptions,
) -> Plan {
    if !selection.is_empty() {
        let statements = match options.for_selection {
            ForSelection::Separate => sql_syntax::statements_in(text, selection, dialect),
            ForSelection::One => sql_syntax::statement_of(text, selection, dialect)
                .into_iter()
                .collect(),
        };
        return run(statements);
    }
    let statements = sql_syntax::split(text, dialect);
    // The statement's own `;` is part of it for the caret.
    let inside = statements.iter().any(|s| {
        s.range.start <= caret
            && text
                .get(s.range.end..caret.max(s.range.end))
                .is_some_and(|tail| tail.bytes().all(|b| b == b';'))
    });
    let candidates = sql_syntax::candidates_at(text, caret, dialect);
    if candidates.is_empty() {
        return Plan::Nothing;
    }
    let statement_candidate = || {
        candidates
            .iter()
            .find(|c| c.kind == CandidateKind::Statement)
    };
    let one = |range: &Range<usize>| sql_syntax::statement_of(text, range.clone(), dialect);
    if inside {
        match options.caret_inside {
            CaretInside::Smallest => run(candidates.first().and_then(|c| one(&c.range))),
            CaretInside::Largest => run(statement_candidate().and_then(|c| one(&c.range))),
            CaretInside::Ask => {
                let choices: Vec<Choice> = candidates
                    .iter()
                    .filter(|c| c.kind != CandidateKind::Script)
                    .filter_map(|c| choice(text, &c.range, one(&c.range)))
                    .collect();
                ask_or_run(choices)
            }
        }
    } else {
        match options.caret_outside {
            CaretOutside::Nothing => Plan::Nothing,
            CaretOutside::Closest => run(statement_candidate().and_then(|c| one(&c.range))),
            CaretOutside::Ask => {
                let mut choices: Vec<Choice> = statement_candidate()
                    .and_then(|c| choice(text, &c.range, one(&c.range)))
                    .into_iter()
                    .collect();
                if let Some(script) = candidates.iter().find(|c| c.kind == CandidateKind::Script) {
                    choices.push(Choice {
                        label: "All statements".to_owned(),
                        statements: sql_syntax::statements_in(text, script.range.clone(), dialect),
                    });
                }
                ask_or_run(choices)
            }
        }
    }
}

fn run(statements: impl IntoIterator<Item = Statement>) -> Plan {
    let statements: Vec<Statement> = statements.into_iter().collect();
    if statements.is_empty() {
        Plan::Nothing
    } else {
        Plan::Run(statements)
    }
}

fn ask_or_run(mut choices: Vec<Choice>) -> Plan {
    match choices.len() {
        0 => Plan::Nothing,
        1 => run(choices.remove(0).statements),
        _ => Plan::Ask(choices),
    }
}

fn choice(text: &str, range: &Range<usize>, statement: Option<Statement>) -> Option<Choice> {
    let statement = statement?;
    let one_line = text[range.clone()]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let label = if one_line.chars().count() > LABEL_CHARS {
        let cut: String = one_line.chars().take(LABEL_CHARS - 1).collect();
        format!("{cut}\u{2026}")
    } else {
        one_line
    };
    Some(Choice {
        label,
        statements: vec![statement],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PG: Dialect = Dialect::Postgres;

    fn options(inside: CaretInside, outside: CaretOutside, sel: ForSelection) -> ExecuteOptions {
        ExecuteOptions {
            caret_inside: inside,
            caret_outside: outside,
            for_selection: sel,
            new_tab: false,
        }
    }

    fn at(script: &str, marker: &str, offset: usize, o: &ExecuteOptions) -> Plan {
        let caret = script.find(marker).unwrap() + offset;
        plan(script, caret, caret..caret, PG, o)
    }

    fn texts(plan: &Plan) -> Vec<String> {
        match plan {
            Plan::Run(s) => s.iter().map(|s| s.text.clone()).collect(),
            Plan::Ask(c) => c.iter().map(|c| c.label.clone()).collect(),
            Plan::Nothing => Vec::new(),
        }
    }

    const SCRIPT: &str = "select 1;\n\nselect * from (select a from t) x;\n\n\n\nselect 3;";

    #[test]
    fn ask_runs_a_plain_statement_at_once() {
        let o = ExecuteOptions::default();
        let p = at(SCRIPT, "select 1", 3, &o);
        assert_eq!(texts(&p), ["select 1"]);
        assert!(matches!(p, Plan::Run(_)));
    }

    #[test]
    fn ask_asks_inside_a_subquery() {
        let o = ExecuteOptions::default();
        let p = at(SCRIPT, "select a", 3, &o);
        assert!(matches!(p, Plan::Ask(_)));
        assert_eq!(
            texts(&p),
            ["select a from t", "select * from (select a from t) x"]
        );
    }

    #[test]
    fn smallest_and_largest() {
        let smallest = options(
            CaretInside::Smallest,
            CaretOutside::Nothing,
            ForSelection::Separate,
        );
        let largest = options(
            CaretInside::Largest,
            CaretOutside::Nothing,
            ForSelection::Separate,
        );
        assert_eq!(
            texts(&at(SCRIPT, "select a", 3, &smallest)),
            ["select a from t"]
        );
        assert_eq!(
            texts(&at(SCRIPT, "select a", 3, &largest)),
            ["select * from (select a from t) x"]
        );
    }

    #[test]
    fn outside_a_statement() {
        // The gap of blank lines between the second and the third statement.
        let caret = SCRIPT.find("x;").unwrap() + 4;
        let blank = |o: &ExecuteOptions| plan(SCRIPT, caret, caret..caret, PG, o);
        let nothing = options(
            CaretInside::Ask,
            CaretOutside::Nothing,
            ForSelection::Separate,
        );
        assert_eq!(blank(&nothing), Plan::Nothing);
        let closest = options(
            CaretInside::Ask,
            CaretOutside::Closest,
            ForSelection::Separate,
        );
        assert_eq!(
            texts(&blank(&closest)),
            ["select * from (select a from t) x"]
        );
        let ask = options(CaretInside::Ask, CaretOutside::Ask, ForSelection::Separate);
        assert_eq!(
            texts(&blank(&ask)),
            ["select * from (select a from t) x", "All statements"]
        );
    }

    #[test]
    fn a_selection_runs_as_separate_statements_or_as_one() {
        let from = SCRIPT.find("select 1").unwrap();
        let to = SCRIPT.find("select 3").unwrap();
        let separate = ExecuteOptions::default();
        let p = plan(SCRIPT, to, from..to, PG, &separate);
        assert_eq!(texts(&p), ["select 1", "select * from (select a from t) x"]);
        let one = options(CaretInside::Ask, CaretOutside::Nothing, ForSelection::One);
        let p = plan(SCRIPT, to, from..to, PG, &one);
        assert_eq!(
            texts(&p),
            ["select 1;\n\nselect * from (select a from t) x"]
        );
    }

    #[test]
    fn an_empty_script_runs_nothing() {
        let o = ExecuteOptions::default();
        assert_eq!(plan("", 0, 0..0, PG, &o), Plan::Nothing);
        assert_eq!(plan("-- x", 2, 2..2, PG, &o), Plan::Nothing);
    }
}
