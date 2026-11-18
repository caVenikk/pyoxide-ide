//! The commands of the query plan: Explain Plan and Explain Analyse of the
//! statement at the caret, and the selection, expansion and view of the
//! plan tab that shows.

use gpui::{App, Entity};

use super::view::{Direction, Mode, PlanView};
use crate::console::{RunMode, target};
use crate::explorer;

ui::ui_commands! {
    COMMANDS, scope = "Database";
    /// Show the plan of the statement at the caret.
    ExplainPlan => "db.console.explain", "Explain Plan",
        "Asks the server for the plan of the statement at the caret of the active SQL editor, or of the first statement of its selection, without running it, and shows it in the Query Plan tab of the Services tool window: a tree with the operation, its object, estimated rows and cost, and the properties of the selected node; the raw text is in the second view of the tab.";
    /// Run the statement and show its plan with the measured rows and times.
    ExplainAnalyze => "db.console.explain-analyze", "Explain Analyse",
        "Like Explain Plan, but the server runs the statement and the plan has the actual rows and times (PostgreSQL). On a read-only data source only a SELECT is run.";
    /// Select a node of the plan.
    SelectPlanNode { path: Vec<usize> } => "db.plan.select", "Select Plan Node",
        "Selects the node of the Query Plan tab at `path`, the indexes of the children from the root (empty: the root); its properties show at the right of the tree.",
        r#"{"path": [0, 1]}"#;
    /// Collapse or expand a node of the plan.
    TogglePlanNode { path: Vec<usize> } => "db.plan.toggle", "Collapse or Expand Plan Node",
        "Collapses the node at `path` of the Query Plan tab (indexes of the children from the root), or expands it when it is collapsed.",
        r#"{"path": [0]}"#;
    PlanUp => "db.plan.up", "Previous Plan Operation", "Selects the previous visible operation.";
    PlanDown => "db.plan.down", "Next Plan Operation", "Selects the next visible operation.";
    PlanLeft => "db.plan.left", "Collapse or Select Parent Operation", "Collapses an expanded operation or selects its parent.";
    PlanRight => "db.plan.right", "Expand or Select Child Operation", "Expands a collapsed operation or selects its first child.";
    SwitchPlanView => "db.plan.switch-view", "Switch Plan View", "Switches between Operations Tree and Raw.";
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
    cx.on_action(|_: &PlanUp, cx| navigate(Direction::Up, cx));
    cx.on_action(|_: &PlanDown, cx| navigate(Direction::Down, cx));
    cx.on_action(|_: &PlanLeft, cx| navigate(Direction::Left, cx));
    cx.on_action(|_: &PlanRight, cx| navigate(Direction::Right, cx));
    cx.on_action(|_: &SwitchPlanView, cx| show_view(None, cx));
    cx.on_action(|a: &ShowPlanAs, cx| {
        let mode = match a.mode.as_str() {
            "tree" => Mode::Tree,
            "text" => Mode::Text,
            _ => return,
        };
        show_view(Some(mode), cx);
    });
}

fn navigate(direction: Direction, cx: &mut App) {
    if let Some(plan) = current(cx) {
        plan.update(cx, |p, cx| p.navigate(direction, cx));
    }
}

fn show_view(mode: Option<Mode>, cx: &mut App) {
    let Some(console) = target(cx) else {
        return;
    };
    let Some(plan) = console.read(cx).current_plan() else {
        return;
    };
    let window = console.read(cx).window;
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, cx| {
            plan.update(cx, |p, cx| {
                p.set_mode(mode.unwrap_or_else(|| p.other_mode()), cx);
                p.focus_mode(window, cx);
            });
        });
    });
}
