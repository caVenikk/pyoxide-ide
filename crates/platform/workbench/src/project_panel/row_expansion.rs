//! The project tree's side of rows whose text does not fit the tool window
//! (the project root with its hint and location, a long name): the
//! mechanism is [`ui::expandable`]; this is what is specific to the file
//! tree, the width of a row's text and how the rest of a row is drawn.

use gpui::{
    AnyElement, App, Context, Div, FontWeight, Hsla, ParentElement as _, Pixels, Point,
    SharedString, Styled as _, Window, div, px,
};
use project::{Node, NodeId};
use themes::{Metric, UiRole};
use ui::expandable::{self, ExpandableItems, Overlay, RowFrame, RowPlace};
use ui::theme::ThemeExt;

use super::{ICON_X, ProjectPanel, TEXT_X, TREE_LEFT};

/// Gap between the root's name and its location.
const LOCATION_GAP: f32 = 6.0;

/// Where the panel keeps its [`ExpandableItems`].
pub(super) fn expansion_access(panel: &mut ProjectPanel) -> &mut ExpandableItems<NodeId> {
    &mut panel.expansion
}

impl ProjectPanel {
    /// Where the text of `node` ends, from the row's left edge, and
    /// remembers it for the state report.
    pub(super) fn measure_label(
        &self,
        id: NodeId,
        node: &Node,
        window: &Window,
        cx: &App,
    ) -> Pixels {
        let size = cx.theme().ui_size();
        let row_text = self.row_text(node);
        let weight = if row_text.bold {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        };
        let mut text = expandable::text_width(&node.entry.name, size, weight, window, cx);
        if let Some(hint) = &row_text.hint {
            text += px(LOCATION_GAP)
                + expandable::text_width(hint, size, FontWeight::NORMAL, window, cx);
        }
        let end = label_x(node, cx) + text;
        self.label_ends.borrow_mut().insert(id, end);
        end
    }

    /// Where the parts of the row of `node` are.
    pub(super) fn row_frame(&self, id: NodeId, node: &Node, window: &Window, cx: &App) -> RowFrame {
        let left = px(TREE_LEFT) + cx.theme().metric(Metric::TreeIndent) * node.depth as f32;
        RowFrame {
            scroll_x: expandable::scrolled(&self.scroll).x,
            view_width: self.expansion.view_width(),
            control_x: if node.entry.is_dir() {
                left
            } else {
                left + px(ICON_X)
            },
            content_end: self.measure_label(id, node, window, cx),
        }
    }

    /// Width of the rows: the widest row's text and what follows it.
    pub(super) fn content_width(&self, window: &Window, cx: &App) -> Pixels {
        let Some(tree) = &self.tree else {
            return px(0.0);
        };
        let rows = tree.rows();
        let indent = f32::from(cx.theme().metric(Metric::TreeIndent));
        expandable::content_width(
            rows.len(),
            |ix| {
                let node = tree.node(rows[ix]);
                let hint = self
                    .row_text(node)
                    .hint
                    .map_or(0, |h| h.chars().count() + 1);
                indent * node.depth as f32 + (node.entry.name.chars().count() + hint) as f32 * 7.0
            },
            |ix| self.measure_label(rows[ix], tree.node(rows[ix]), window, cx),
        )
    }

    /// The row at a position of the window.
    pub(super) fn row_at_position(&self, position: Point<Pixels>, cx: &App) -> Option<NodeId> {
        let tree = self.tree.as_ref()?;
        let row = expandable::row_at(
            position,
            self.expansion.view(),
            self.scroll_top(),
            cx.theme().metric(Metric::RowHeight),
            tree.rows().len(),
        )?;
        tree.rows().get(row).copied()
    }

    /// The rest of the row the pointer is at (or the selected row), over
    /// the tree's edge.
    pub(super) fn expansion_overlay(
        &self,
        focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.expansion.reset_hint();
        let id = *self.expansion.shown(focused, self.selected.as_ref())?;
        if self.context_menu.is_some() {
            return None;
        }
        let tree = self.tree.as_ref()?;
        let row = tree.row_of(id)?;
        let node = tree.node(id);
        let frame = self.row_frame(id, node, window, cx);
        if !frame.is_cut() {
            return None;
        }
        let theme = cx.theme();
        let row_height = theme.metric(Metric::RowHeight);
        let overlay = Overlay {
            row_top: row_height * row as f32 - self.scroll_top(),
            row_height,
            frame,
            background: theme.color(UiRole::ToolWindow),
            border: theme.color(UiRole::Border),
            radius: theme.metric(Metric::CornerRadius),
            row: self.row(id, row, focused, RowPlace::Rest, window, cx),
        };
        self.expansion
            .overlay(overlay, window, expansion_access, cx)
    }

    /// The row's name and its grey hint (a content root's kind of root,
    /// the project root's location), as the row draws it.
    pub(super) fn row_label(&self, node: &Node, text: Hsla, cx: &App) -> Div {
        let row_text = self.row_text(node);
        let mut label = div()
            .flex()
            .flex_row()
            .gap(px(LOCATION_GAP))
            .whitespace_nowrap()
            .text_color(text)
            .child(SharedString::from(node.entry.name.clone()));
        if row_text.bold {
            label = label.font_weight(FontWeight::SEMIBOLD);
        }
        if let Some(hint) = row_text.hint {
            label = label.child(
                div()
                    .font_weight(FontWeight::NORMAL)
                    .text_color(cx.theme().color(UiRole::TextMuted))
                    .child(hint),
            );
        }
        label
    }

    /// The row whose rest shows, relative to the project (for
    /// `workbench.state`).
    pub(super) fn expansion_json(&self, focused: bool, cx: &App) -> serde_json::Value {
        let _ = cx;
        let (Some(id), Some(tree), Some(project)) = (
            self.expansion
                .shown(focused, self.selected.as_ref())
                .copied(),
            &self.tree,
            &self.project,
        ) else {
            return serde_json::Value::Null;
        };
        if self.context_menu.is_some() || !self.overflow_json(id) || !self.expansion.is_showing() {
            return serde_json::Value::Null;
        }
        let path = &tree.node(id).entry.path;
        let rel = path
            .strip_prefix(project.root())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        serde_json::json!(rel)
    }

    /// The row's text ends past the tree's edge (for `workbench.state`).
    pub(super) fn overflow_json(&self, id: NodeId) -> bool {
        let width = self.expansion.view_width();
        let end = self.label_ends.borrow().get(&id).copied();
        let scroll = expandable::scrolled(&self.scroll).x;
        width > px(0.0) && end.is_some_and(|end| end + px(expandable::CELL_PAD) > scroll + width)
    }
}

/// Where the text of a row starts, from the row's left edge.
fn label_x(node: &Node, cx: &App) -> Pixels {
    px(TREE_LEFT) + cx.theme().metric(Metric::TreeIndent) * node.depth as f32 + px(ICON_X + TEXT_X)
}
