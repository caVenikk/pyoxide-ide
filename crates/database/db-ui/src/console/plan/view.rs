//! The Query Plan tab of the Services tool window: the plan as a table with
//! a tree in its first column and, at its right, the properties of the
//! selected node (DB72, DB73); a toolbar at the left switches to the raw
//! text.

use std::collections::HashSet;

use console::ContentKind;
use console_ui::ConsoleView;
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
};
use serde_json::{Value as Json, json};
use themes::UiRole;
use ui::components::{Tooltip, icon_button, tree_arrow};
use ui::icons::Icon;
use ui::theme::ThemeExt;

use super::{Col, NodeKind, Plan, number};
use crate::icons;

const ROW: f32 = 26.0;
const INDENT: f32 = 16.0;
const DETAILS_WIDTH: f32 = 300.0;
const PROPERTY_WIDTH: f32 = 130.0;
const TOOLBAR_WIDTH: f32 = 32.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Tree,
    Text,
}

pub(crate) struct PlanView {
    pub plan: Plan,
    collapsed: HashSet<Vec<usize>>,
    selected: Vec<usize>,
    mode: Mode,
    raw: Entity<ConsoleView>,
    focus: FocusHandle,
}

impl Focusable for PlanView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PlanView {
    pub fn new(plan: Plan, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let raw = cx.new(|cx| {
            let mut view = ConsoleView::new(window, cx);
            view.set_key_context("DatabaseOutput");
            view
        });
        raw.update(cx, |view, cx| {
            view.model()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_str(ContentKind::Normal, &format!("{}\n", plan.raw));
            view.flush(cx);
            // The answer is read from its first line.
            view.editor().update(cx, |e, cx| e.go_to(0, None, cx));
        });
        Self {
            plan,
            collapsed: HashSet::new(),
            selected: Vec::new(),
            mode: Mode::Tree,
            raw,
            focus: cx.focus_handle(),
        }
    }

    pub fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    pub fn select(&mut self, path: Vec<usize>, cx: &mut Context<Self>) {
        if self.plan.node(&path).is_some() {
            self.selected = path;
            cx.notify();
        }
    }

    /// Opens a collapsed node, collapses an open one that has children.
    pub fn toggle(&mut self, path: Vec<usize>, cx: &mut Context<Self>) {
        if self.plan.node(&path).is_none_or(|n| n.children.is_empty()) {
            return;
        }
        if !self.collapsed.remove(&path) {
            // The selection stays on a row that is still shown.
            if self.selected.starts_with(&path) {
                self.selected = path.clone();
            }
            self.collapsed.insert(path);
        }
        cx.notify();
    }

    pub fn state_json(&self, cx: &App) -> Json {
        let lines = self.plan.lines(&self.collapsed);
        let value = |n: Option<f64>| n.map(number);
        json!({
            "mode": match self.mode { Mode::Tree => "tree", Mode::Text => "text" },
            "analyze": self.plan.analyze,
            "columns": self.plan.columns().iter().map(|c| c.title()).collect::<Vec<_>>(),
            "selected": self.selected,
            "rows": lines.iter().map(|l| json!({
                "path": l.path,
                "depth": l.path.len(),
                "operation": l.node.operation,
                "params": l.node.params,
                "rows": value(l.node.rows),
                "actual_rows": value(l.node.actual_rows),
                "total_cost": value(l.node.total_cost),
                "startup_cost": value(l.node.startup_cost),
                "actual_total_time": value(l.node.actual_total_time),
                "actual_startup_time": value(l.node.actual_startup_time),
                "raw_desc": l.node.raw_desc,
                "has_children": !l.node.children.is_empty(),
                "collapsed": self.collapsed.contains(&l.path),
            })).collect::<Vec<_>>(),
            "details": self.plan.node(&self.selected).map(|n| {
                n.props.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>()
            }),
            "raw": self.raw.read(cx).text(cx),
        })
    }

    fn cell(&self, node: &super::PlanNode, col: Col) -> String {
        let n = |v: Option<f64>| v.map(number).unwrap_or_default();
        match col {
            Col::Operation => node.operation.clone(),
            Col::Params => node.params.clone(),
            Col::Rows => n(node.rows),
            Col::ActualRows => n(node.actual_rows),
            Col::TotalCost => n(node.total_cost),
            Col::ActualTotalTime => n(node.actual_total_time),
            Col::StartupCost => n(node.startup_cost),
            Col::ActualStartupTime => n(node.actual_startup_time),
            Col::RawDesc => node.raw_desc.clone(),
        }
    }

    fn node_icon(kind: NodeKind) -> gpui::AnyElement {
        let icon = match kind {
            NodeKind::Statement => icons::PLAN_SELECT,
            NodeKind::Scan => icons::TABLE,
            NodeKind::IndexScan => icons::INDEX,
            NodeKind::Join => icons::PLAN_JOIN,
            NodeKind::Transform => icons::PLAN_TRANSFORM,
            NodeKind::Other => icons::PLAN_OPERATION,
        };
        Icon::new(icon).size(px(16.0)).into_any_element()
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let button = |id: &'static str, icon, tip: &'static str, on: bool, mode: Mode| {
            let cx_ref: &App = cx;
            icon_button(id, icon, px(24.0), px(16.0), on, cx_ref)
                .debug_selector(move || id.to_owned())
                .tooltip(Tooltip::text(tip, None))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_mode(mode, cx)))
        };
        div()
            .id("plan-toolbar")
            .flex()
            .flex_col()
            .flex_none()
            .items_center()
            .w(px(TOOLBAR_WIDTH))
            .pt(px(2.0))
            .gap(px(2.0))
            .child(button(
                "plan-show-tree",
                icons::PLAN,
                "Query Plan",
                self.mode == Mode::Tree,
                Mode::Tree,
            ))
            .child(button(
                "plan-show-text",
                icons::PLAN_TEXT,
                "Raw Text",
                self.mode == Mode::Text,
                Mode::Text,
            ))
            .into_any_element()
    }

    fn render_table(&self, focused: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let border = theme.color(UiRole::Border);
        let muted = theme.color(UiRole::TextMuted);
        let selected_bg = theme.color(if focused {
            UiRole::Selection
        } else {
            UiRole::SelectionInactive
        });
        let hover = theme.color(UiRole::Hover);
        let cols = self.plan.columns();
        let total: f32 = cols.iter().map(|c| c.width()).sum();
        let header = div()
            .flex()
            .flex_row()
            .flex_none()
            .h(px(ROW))
            .min_w(px(total))
            .border_b_1()
            .border_color(border)
            .text_color(muted)
            .children(cols.iter().map(|c| {
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .w(px(c.width()))
                    .px(px(8.0))
                    .when(c.numeric(), |d| d.justify_end())
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .child(c.title())
            }));
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        for (ix, line) in self.plan.lines(&self.collapsed).into_iter().enumerate() {
            let depth = line.path.len();
            let is_selected = line.path == self.selected;
            let has_children = !line.node.children.is_empty();
            let collapsed = self.collapsed.contains(&line.path);
            let path = line.path.clone();
            let mut row = div()
                .id(gpui::SharedString::from(format!("plan-row-{ix}")))
                .debug_selector(move || format!("plan-row-{ix}"))
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .h(px(ROW))
                .min_w(px(total))
                .when(is_selected, |d| d.bg(selected_bg))
                .when(!is_selected, |d| d.hover(move |s| s.bg(hover)))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    window.focus(&this.focus);
                    this.select(path.clone(), cx)
                }));
            for col in &cols {
                let text = self.cell(line.node, *col);
                let mut cell = div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap(px(4.0))
                    .w(px(col.width()))
                    .h_full()
                    .px(px(8.0))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .when(col.numeric(), |d| d.justify_end());
                if *col == Col::Operation {
                    let arrow_path = line.path.clone();
                    let slot = div().flex_none().w(px(16.0)).h(px(16.0));
                    let slot = if has_children {
                        slot.child(tree_arrow(!collapsed, cx)).on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle(arrow_path.clone(), cx);
                            }),
                        )
                    } else {
                        slot
                    };
                    cell = cell
                        .pl(px(4.0 + depth as f32 * INDENT))
                        .child(slot)
                        .child(Self::node_icon(line.node.kind));
                }
                row = row.child(cell.child(text).text_ellipsis());
            }
            rows.push(row.into_any_element());
        }
        div()
            .id("plan-table")
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .overflow_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(total))
                    .child(header)
                    .children(rows),
            )
            .into_any_element()
    }

    /// The properties of the selected node.
    fn render_details(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let border = theme.color(UiRole::Border);
        let muted = theme.color(UiRole::TextMuted);
        let props = self
            .plan
            .node(&self.selected)
            .map(|n| n.props.clone())
            .unwrap_or_default();
        let cell = |w: Option<f32>| {
            let d = div()
                .flex()
                .items_center()
                .h_full()
                .px(px(8.0))
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis();
            match w {
                Some(w) => d.flex_none().w(px(w)),
                None => d.flex_1().min_w(px(0.0)),
            }
        };
        let header = div()
            .flex()
            .flex_row()
            .flex_none()
            .h(px(ROW))
            .border_b_1()
            .border_color(border)
            .text_color(muted)
            .child(cell(Some(PROPERTY_WIDTH)).child("Property"))
            .child(cell(None).child("Value"));
        div()
            .id("plan-details")
            .debug_selector(|| "plan-details".to_owned())
            .flex()
            .flex_col()
            .flex_none()
            .w(px(DETAILS_WIDTH))
            .h_full()
            .border_l_1()
            .border_color(border)
            .overflow_y_scroll()
            .child(header)
            .children(props.into_iter().enumerate().map(|(ix, (k, v))| {
                div()
                    .id(gpui::SharedString::from(format!("plan-prop-{ix}")))
                    .flex()
                    .flex_row()
                    .flex_none()
                    .h(px(ROW))
                    .child(cell(Some(PROPERTY_WIDTH)).child(k))
                    .child(cell(None).child(v))
            }))
            .into_any_element()
    }
}

impl Render for PlanView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus.contains_focused(window, cx);
        let toolbar = self.render_toolbar(cx);
        let body = match self.mode {
            Mode::Tree => div()
                .flex()
                .flex_row()
                .size_full()
                .child(self.render_table(focused, cx))
                .child(self.render_details(cx))
                .into_any_element(),
            Mode::Text => div().size_full().child(self.raw.clone()).into_any_element(),
        };
        div()
            .id("plan-view")
            .debug_selector(|| "plan-view".to_owned())
            .track_focus(&self.focus)
            .flex()
            .flex_row()
            .size_full()
            .child(toolbar)
            .child(div().flex_1().min_w(px(0.0)).h_full().child(body))
    }
}
