//! What happens between Execute and the session: the warning before a
//! `DELETE` or `UPDATE` without `WHERE` and the Parameters dialog. Values
//! typed there are bound to the statements, never put into their text.

use std::collections::HashMap;

use console::ContentKind;
use gpui::{Context, Window};
use serde_json::{Value as Json, json};
use sql_syntax::{Danger, Statement};

use super::Console;
use crate::console::{params, params_dialog};
use crate::query_settings;

/// The statements the unsafe-query banner waits for.
pub(crate) struct UnsafeRun {
    pub statements: Vec<Statement>,
    pub message: String,
    pub explain: Option<bool>,
}

pub(crate) fn danger_message(danger: Danger) -> &'static str {
    match danger {
        Danger::DeleteWithoutWhere => {
            "Unsafe query: 'Delete' statement without 'where' clears all data in the table"
        }
        Danger::UpdateWithoutWhere => {
            "Unsafe query: 'Update' statement without 'where' updates all data in the table"
        }
    }
}

impl Console {
    /// Runs `statements` after the checks: a statement without `WHERE`
    /// first waits for the banner's answer, then the parameters are asked.
    pub fn submit(
        &mut self,
        statements: Vec<Statement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_run(statements, None, window, cx);
    }

    /// Carries an Explain mode through warning and parameter dialogs without
    /// marking a plan as running before the user confirms the request.
    pub(super) fn submit_run(
        &mut self,
        statements: Vec<Statement>,
        explain: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if statements.is_empty() {
            return;
        }
        if self.running.is_some() {
            self.print(ContentKind::Error, "A statement is still running", cx);
            return;
        }
        let settings = query_settings::get(cx);
        let danger = statements.iter().find_map(|s| s.danger);
        if explain != Some(false)
            && settings.warn_unsafe_queries
            && let Some(danger) = danger
        {
            let message = danger_message(danger).to_owned();
            self.print(ContentKind::Error, &message, cx);
            self.unsafe_run = Some(UnsafeRun {
                statements,
                message,
                explain,
            });
            cx.notify();
            return;
        }
        self.review_parameters(statements, explain, window, cx);
    }

    /// The banner's Execute (and Execute and Suppress, which also turns the
    /// warning off).
    pub fn confirm_unsafe(&mut self, suppress: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.unsafe_run.take() else {
            return;
        };
        if suppress {
            let mut settings = query_settings::get(cx);
            settings.warn_unsafe_queries = false;
            query_settings::set(cx, &settings);
        }
        cx.notify();
        self.review_parameters(run.statements, run.explain, window, cx);
    }

    /// The banner's close button: nothing runs.
    pub fn dismiss_unsafe(&mut self, cx: &mut Context<Self>) {
        if self.unsafe_run.take().is_some() {
            cx.notify();
        }
    }

    /// The Parameters button: asks for the values of the statement at the
    /// caret whatever the setting says, then runs it. Nothing happens for
    /// a statement without parameters.
    pub fn edit_parameters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let statements = self.statements_at_caret(cx);
        if self.running.is_some() || statements.is_empty() {
            return;
        }
        self.new_tab_run = query_settings::get(cx).execute.new_tab;
        self.ask_parameters(statements, None, true, window, cx);
    }

    /// Asks for the values the statements need (always when the setting is
    /// on, else when one is missing), then runs them with the values bound.
    fn review_parameters(
        &mut self,
        statements: Vec<Statement>,
        explain: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask_parameters(statements, explain, false, window, cx);
    }

    fn ask_parameters(
        &mut self,
        statements: Vec<Statement>,
        explain: Option<bool>,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dialect = self.dialect();
        let slots = params::slots(&statements, dialect);
        if slots.is_empty() {
            if !force {
                self.run_with(statements, &HashMap::new(), explain, cx);
            }
            return;
        }
        let review = force || query_settings::get(cx).review_parameters;
        let missing = slots
            .iter()
            .any(|s| !self.param_values.contains_key(&s.key));
        if !review && !missing {
            let values = self.param_values.clone();
            self.run_with(statements, &values, explain, cx);
            return;
        }
        let this = cx.entity().downgrade();
        let remembered = self.param_values.clone();
        params_dialog::ask(slots, &remembered, window, cx, move |values, cx| {
            let Some(values) = values else {
                return;
            };
            let Some(console) = this.upgrade() else {
                return;
            };
            console.update(cx, |c, cx| {
                c.param_values.extend(values.clone());
                c.run_with(statements, &values, explain, cx);
            });
        });
    }

    fn run_with(
        &mut self,
        statements: Vec<Statement>,
        values: &HashMap<String, String>,
        explain: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let dialect = self.dialect();
        let jobs: Vec<_> = statements
            .iter()
            .enumerate()
            .map(|(i, s)| params::bind(s, i, dialect, values))
            .collect();
        if let Some(analyze) = explain {
            if let Some((statement, job)) = statements.into_iter().zip(jobs).next() {
                self.start_explain(statement, job, analyze, cx);
            }
        } else {
            self.run_jobs(jobs, cx);
        }
    }

    pub(super) fn safety_json(&self) -> Json {
        json!({
            "unsafe": self.unsafe_run.as_ref().map(|u| &u.message),
            "parameters": self.param_values,
        })
    }
}
