//! Explain Plan and Explain Analyze of a console: the statements of
//! `sql_syntax::explain_statements` run as a job like any other, and the
//! rows they answer with become a Query Plan tab instead of a grid.

use db_core::Row;
use db_session::JobStatement;
use sql_syntax::{ExplainOptions, Statement, StatementKind};

use gpui::{AppContext as _, Context, Window};

use super::{Console, ContentKind, Pane, ResultTab};
use crate::console::plan::{Plan, view::PlanView};

/// What Run does with the statement at the caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunMode {
    Execute,
    /// The plan of the statement; with `analyze` the statement runs and the
    /// plan has the measured rows and times.
    Explain { analyze: bool },
}

/// The explain the console is running.
pub(super) struct Run {
    pub analyze: bool,
    /// The statement the plan is of.
    pub statement: String,
}

/// The title of the plan's tab.
pub(crate) const PLAN_TITLE: &str = "Query Plan";

impl Console {
    pub fn run_mode(&mut self, mode: RunMode, cx: &mut Context<Self>) {
        match mode {
            RunMode::Execute => self.execute_at_caret(cx),
            RunMode::Explain { analyze } => {
                if let Some(statement) = self.caret_statements(cx).into_iter().next() {
                    self.explain(statement, analyze, cx);
                }
            }
        }
    }

    /// Asks for the plan of `statement`.
    pub fn explain(&mut self, statement: Statement, analyze: bool, cx: &mut Context<Self>) {
        if self.running.is_some() {
            self.print(ContentKind::Error, "A statement is still running", cx);
            return;
        }
        let dialect = self.dialect();
        if analyze {
            if !sql_syntax::supports_analyze(dialect) {
                self.print(
                    ContentKind::Error,
                    "Explain Analyze is not available for this data source",
                    cx,
                );
                return;
            }
            // The statement runs: a read-only data source only lets a
            // query do that.
            let query = matches!(statement.kind, StatementKind::Select | StatementKind::With)
                && !statement.writes;
            if (self.read_only || self.handle.options().read_only) && !query {
                self.print(
                    ContentKind::Error,
                    "Explain Analyze runs the statement: on a read-only data source it is allowed for SELECT only",
                    cx,
                );
                return;
            }
        }
        let texts = sql_syntax::explain_statements(
            &statement.text,
            dialect,
            ExplainOptions { analyze },
        );
        if texts.is_empty() {
            return;
        }
        let statements: Vec<JobStatement> = texts
            .into_iter()
            .map(|text| {
                let class = sql_syntax::classify_text(&text, dialect);
                JobStatement {
                    text,
                    range: statement.range.clone(),
                    kind: class.kind,
                    // The analyzed statement does what it does.
                    writes: class.writes || (analyze && statement.writes),
                    params: Vec::new(),
                }
            })
            .collect();
        self.explain = Some(Run {
            analyze,
            statement: statement.text,
        });
        self.start(statements, cx);
        if self.running.is_none() {
            self.explain = None;
        }
    }

    /// The answer of `EXPLAIN` as a tab; `false` when no explain is
    /// running (the rows are an ordinary result).
    pub(super) fn show_plan(
        &mut self,
        rows: &[Row],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(run) = &self.explain else {
            return false;
        };
        let plan = Plan::from_rows(self.dialect(), &run.statement, rows, run.analyze);
        let view = cx.new(|cx| PlanView::new(plan, window, cx));
        let tab = ResultTab {
            title: PLAN_TITLE.into(),
            pinned: false,
            renamed: false,
            grid: None,
            plan: Some(view),
            result: None,
        };
        let reuse = self
            .tabs
            .iter()
            .position(|t| t.plan.is_some() && !t.pinned && !t.renamed);
        let index = match reuse {
            Some(ix) => {
                self.tabs[ix] = tab;
                ix
            }
            None => {
                self.tabs.push(tab);
                self.tabs.len() - 1
            }
        };
        self.pane = Pane::Result(index);
        cx.notify();
        true
    }

    /// The plan the console shows now, if its pane is one.
    pub fn current_plan(&self) -> Option<gpui::Entity<PlanView>> {
        match self.pane {
            Pane::Result(i) => self.tabs.get(i).and_then(|t| t.plan.clone()),
            Pane::Output => None,
        }
    }
}
