//! The commands of the query plan: Explain Plan and Explain Analyze of the
//! statement at the caret, and the selection, expansion and view of the
//! plan tab that shows.

use gpui::{App, Entity};

use super::view::{Mode, PlanView};
use crate::console::{RunMode, target};
use crate::explorer;

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Show the plan of the statement at the caret.
    ExplainPlan => "db.console.explain", "Explain Plan",
        "Asks the server for the plan of the statement at the caret of the active SQL editor, or of the first statement of its selection, without running it, and shows it in the Query Plan tab of the Services tool window: a tree with the operation, its object, estimated rows and cost, and the properties of the selected node; the raw text is in the second view of the tab.";
    /// Run the statement and show its plan with the measured rows and times.
    ExplainAnalyze => "db.console.explain-analyze", "Explain Analyze",
        "Like Explain Plan, but the server runs the statement and the plan has the actual rows and times (PostgreSQL). On a read-only data source only a SELECT is run.";
    /// Select a node of the plan.
    SelectPlanNode { path: Vec<usize> } => "db.plan.select", "Select Plan Node",
        "Selects the node of the Query Plan tab at `path`, the indexes of the children from the root (empty: the root); its properties show at the right of the tree.",
        r#"{"path": [0, 1]}"#;
    /// Collapse or expand a node of the plan.
    TogglePlanNode { path: Vec<usize> } => "db.plan.toggle", "Collapse or Expand Plan Node",
        "Collapses the node at `path` of the Query Plan tab (indexes of the children from the root), or expands it when it is collapsed.",
        r#"{"path": [0]}"#;
    /// Switch the plan between the tree and the raw text.
    ShowPlanAs { mode: String } => "db.plan.view", "Show Plan As",
        "Shows the Query Plan tab as `tree` or as the raw `text` of the server's answer.",
        r#"{"mode": "text"}"#;
}

/// The plan the active console shows.
fn current(cx: &App) -> Option<Entity<PlanView>> {
    target(cx)?.read(cx).current_plan()
}

pub(crate) fn init(cx: &mut App) {
    ui::command::register(cx, COMMANDS);
    cx.on_action(|_: &ExplainPlan, cx| {
        explorer::with_target(cx, |e, window, cx| {
            e.run_at_caret(RunMode::Explain { analyze: false }, window, cx)
        });
    });
    cx.on_action(|_: &ExplainAnalyze, cx| {
        explorer::with_target(cx, |e, window, cx| {
            e.run_at_caret(RunMode::Explain { analyze: true }, window, cx)
        });
    });
    cx.on_action(|a: &SelectPlanNode, cx| {
        if let Some(plan) = current(cx) {
            let path = a.path.clone();
            plan.update(cx, |p, cx| p.select(path, cx));
        }
    });
    cx.on_action(|a: &TogglePlanNode, cx| {
        if let Some(plan) = current(cx) {
            let path = a.path.clone();
            plan.update(cx, |p, cx| p.toggle(path, cx));
        }
    });
    cx.on_action(|a: &ShowPlanAs, cx| {
        let mode = match a.mode.as_str() {
            "tree" => Mode::Tree,
            "text" => Mode::Text,
            _ => return,
        };
        if let Some(plan) = current(cx) {
            plan.update(cx, |p, cx| p.set_mode(mode, cx));
        }
    });
}
