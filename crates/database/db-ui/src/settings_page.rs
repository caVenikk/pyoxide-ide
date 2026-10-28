//! Settings > Database: the group and its Query Execution page, with the
//! options of running statements that the console uses today (review of
//! parameters, the execution time in the editor, the warning before an
//! unsafe query). The other options of the reference's page appear with the
//! features they switch.

use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, FocusHandle, IntoElement, Render, Window, div,
    prelude::*, px,
};
use serde_json::{Value as Json, json};
use settings_ui::{PageContext, PageSpec, Scope, SettingsPage};

use crate::query_settings::{self, QueryExecutionSettings};

pub const DATABASE: &str = "database";
pub const QUERY_EXECUTION: &str = "database.query-execution";

const REVIEW: &str = "Review parameters before execution";
const SHOW_TIME: &str = "Show query execution time in the editor after query completion";
const WARN_UNSAFE: &str = "Show warning before running potentially unsafe queries";
const SEARCH_TERMS: &[&str] = &[REVIEW, SHOW_TIME, WARN_UNSAFE];

const ROW_HEIGHT: f32 = 22.0;
const ROW_GAP: f32 = 6.0;

/// The keys of `settings.edit`.
mod field {
    pub const REVIEW: &str = "review-parameters";
    pub const SHOW_TIME: &str = "show-execution-time";
    pub const WARN_UNSAFE: &str = "warn-unsafe-queries";
}

pub fn pages() -> Vec<PageSpec> {
    vec![
        PageSpec {
            weight: 100,
            ..PageSpec::group(DATABASE, None, "Database")
        },
        PageSpec {
            id: QUERY_EXECUTION,
            parent: Some(DATABASE),
            title: "Query Execution",
            weight: 100,
            scope: Scope::Application,
            search_terms: SEARCH_TERMS,
            description: None,
            create: Some(settings_ui::page::create(QueryExecutionPage::new)),
        },
    ]
}

/// Database > Query Execution.
pub struct QueryExecutionPage {
    draft: QueryExecutionSettings,
    focus: FocusHandle,
}

impl QueryExecutionPage {
    fn new(_: &PageContext, _: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            draft: query_settings::get(cx),
            focus: cx.focus_handle(),
        }
    }

    fn toggle(&mut self, name: &str, cx: &mut Context<Self>) {
        match name {
            field::REVIEW => self.draft.review_parameters = !self.draft.review_parameters,
            field::SHOW_TIME => self.draft.show_execution_time = !self.draft.show_execution_time,
            field::WARN_UNSAFE => self.draft.warn_unsafe_queries = !self.draft.warn_unsafe_queries,
            _ => return,
        }
        cx.notify();
    }

    fn check(
        &self,
        name: &'static str,
        checked: bool,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = ui::form::check_row(
            ElementId::Name(format!("database-row:{name}").into()),
            ElementId::Name(format!("database-check:{name}").into()),
            checked,
            div().whitespace_nowrap().child(label),
            cx,
        )
        .debug_selector(move || format!("database:{name}"))
        .min_h(px(ROW_HEIGHT))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(name, cx)));
        settings_ui::search::spotlight(label, row, cx)
    }
}

impl SettingsPage for QueryExecutionPage {
    fn is_modified(&self, cx: &App) -> bool {
        self.draft != query_settings::get(cx)
    }

    fn apply(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        query_settings::set(cx, &self.draft);
        cx.notify();
        Ok(())
    }

    fn reset(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.draft = query_settings::get(cx);
        cx.notify();
    }

    fn focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.focus.clone())
    }

    fn state(&self, _: &App) -> Json {
        json!({
            field::REVIEW: self.draft.review_parameters,
            field::SHOW_TIME: self.draft.show_execution_time,
            field::WARN_UNSAFE: self.draft.warn_unsafe_queries,
        })
    }

    fn edit(
        &mut self,
        name: &str,
        value: &Json,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let flag = value
            .as_bool()
            .ok_or_else(|| format!("`{name}` takes true or false"))?;
        match name {
            field::REVIEW => self.draft.review_parameters = flag,
            field::SHOW_TIME => self.draft.show_execution_time = flag,
            field::WARN_UNSAFE => self.draft.warn_unsafe_queries = flag,
            _ => return Err(format!("no field `{name}` on this page")),
        }
        cx.notify();
        Ok(())
    }
}

impl Render for QueryExecutionPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let draft = self.draft;
        div()
            .id("database-query-execution-page")
            .debug_selector(|| "database-query-execution-page".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .gap(px(ROW_GAP))
            .child(self.check(field::REVIEW, draft.review_parameters, REVIEW, cx))
            .child(self.check(field::SHOW_TIME, draft.show_execution_time, SHOW_TIME, cx))
            .child(self.check(
                field::WARN_UNSAFE,
                draft.warn_unsafe_queries,
                WARN_UNSAFE,
                cx,
            ))
    }
}
