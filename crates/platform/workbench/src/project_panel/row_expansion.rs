//! Rows whose text does not fit the tool window (the project root with its
//! location, a long name), drawn as the reference IDE's trees draw them:
//!
//! - The row's selection and hover run out of the visible part of the
//!   tree: they reach its right edge but for a pixel, with square right
//!   corners (the tree is wider than the view and the rounded end of the
//!   selection is out of sight).
//! - Pointing at the row, or selecting it while the tree has the focus,
//!   shows the rest of the row over the tree's edge, as the reference's
//!   `TreeExpandableItemsHandler` and `AbstractExpandableItemsHandler`
//!   (IntelliJ Community, Apache-2.0) do: the part beyond the edge,
//!   continuing the row without a seam (it starts where the row's
//!   selection ends, on the same background, with the text at the same
//!   place), framed by a 1 px border outside the row above, below and at
//!   the rounded right end, none where it joins the row. While it shows,
//!   the row keeps its hover background. It follows the pointer and the selection 10 ms
//!   later, stays while the pointer is on it (clicks on it are clicks on
//!   the row), goes when the pointer leaves both the rows and it, and shows
//!   only for a row the tree shows whole.
//!
//! A row that fits is drawn as before.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, Context, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Point, SharedString, StatefulInteractiveElement as _, Styled as _,
    Task, TextRun, Window, anchored, deferred, div, point, px, size,
};
use project::{Node, NodeId};
use themes::{Metric, UiRole};
use ui::theme::ThemeExt;

use super::{ICON_X, ProjectPanel, TEXT_X, TREE_LEFT};

/// What is left of the tree's width right of a wide row's selection.
pub(super) const WIDE_MARGIN: f32 = 1.0;
/// Space after the text in the shown rest of a row.
const EXPANSION_PAD: f32 = 4.0;
/// Width of the border of the shown rest of a row.
const HINT_BORDER: f32 = 1.0;
/// Gap between the root's name and its location.
const LOCATION_GAP: f32 = 6.0;
/// The rest of a row follows the pointer and the selection this much later
/// (`AbstractExpandableItemsHandler`'s update alarm).
pub(super) const EXPANSION_DELAY: Duration = Duration::from_millis(10);

/// Which row shows its rest, and where the drawn rows' text ends.
#[derive(Default)]
pub(super) struct RowExpansion {
    /// The row the pointer is over, or the row last selected.
    key: Option<NodeId>,
    /// `key` came from the selection: it shows while the tree has the
    /// focus and the row stays selected.
    by_selection: bool,
    /// Where each drawn row's text ends, from the row's left edge.
    label_ends: RefCell<HashMap<NodeId, Pixels>>,
    /// Where the rest of a row was drawn last (window coordinates).
    hint: Cell<Option<Bounds<Pixels>>>,
    /// The change of `key` waiting for [`EXPANSION_DELAY`].
    pending: Option<Task<()>>,
}

impl RowExpansion {
    pub(super) fn clear(&mut self) {
        self.key = None;
        self.by_selection = false;
        self.label_ends.borrow_mut().clear();
        self.hint.set(None);
        self.pending = None;
    }

    fn hint_contains(&self, position: Point<Pixels>) -> bool {
        self.hint.get().is_some_and(|b| b.contains(&position))
    }
}

impl ProjectPanel {
    /// Shows the rest of `key` (or of nothing) after [`EXPANSION_DELAY`];
    /// a change back to the current row cancels a waiting one.
    fn expand_later(&mut self, key: Option<NodeId>, by_selection: bool, cx: &mut Context<Self>) {
        if (key, by_selection) == (self.expansion.key, self.expansion.by_selection) {
            self.expansion.pending = None;
            return;
        }
        let timer = cx.background_executor().timer(EXPANSION_DELAY);
        self.expansion.pending = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                this.expansion.key = key;
                this.expansion.by_selection = by_selection;
                this.expansion.pending = None;
                cx.notify();
            });
        }));
    }

    /// The selection moved to `id`.
    pub(super) fn selection_moved(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.expand_later(Some(id), true, cx);
    }

    /// Where the text of `node` ends, from the row's left edge, and
    /// remembers it for the row's expansion.
    pub(super) fn measure_label(
        &self,
        id: NodeId,
        node: &Node,
        window: &Window,
        cx: &App,
    ) -> Pixels {
        let theme = cx.theme();
        let size = theme.ui_size();
        let width = |text: &str, weight: FontWeight| {
            if text.is_empty() {
                return px(0.0);
            }
            let mut font = gpui::font(theme.ui_family());
            font.weight = weight;
            let run = TextRun {
                len: text.len(),
                font,
                color: theme.color(UiRole::Text),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(SharedString::from(text.to_owned()), size, &[run], None)
                .width
        };
        let mut text = if node.depth == 0 {
            width(&node.entry.name, FontWeight::SEMIBOLD)
        } else {
            width(&node.entry.name, FontWeight::NORMAL)
        };
        if node.depth == 0
            && let Some(project) = &self.project
        {
            text += px(LOCATION_GAP) + width(&project.display_root(), FontWeight::NORMAL);
        }
        let end = label_x(node, cx) + text;
        self.expansion.label_ends.borrow_mut().insert(id, end);
        end
    }

    /// Width of the rows' area, zero before it was first drawn.
    fn rows_width(&self) -> Pixels {
        self.rows_bounds.get().size.width
    }

    /// A row whose text ends past the normal right end of the selection:
    /// its selection and hover run to the tree's edge.
    pub(super) fn is_wide(&self, label_end: Pixels) -> bool {
        let width = self.rows_width();
        width > px(0.0) && label_end > width - px(TREE_LEFT)
    }

    /// The row's text is cut by the tree's edge (as measured when the row
    /// was last drawn).
    fn is_overflowing(&self, id: NodeId) -> bool {
        let width = self.rows_width();
        let end = self.expansion.label_ends.borrow().get(&id).copied();
        width > px(0.0) && end.is_some_and(|end| end > width)
    }

    /// The pointer moved over the rows (window coordinates).
    pub(super) fn pointer_over_rows(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let id = self.row_at(position, cx);
        self.expand_later(id, false, cx);
    }

    /// The pointer left the rows or the rest of a row: the rest goes
    /// unless the pointer is on the other one.
    pub(super) fn pointer_left(&mut self, window: &Window, cx: &mut Context<Self>) {
        let position = window.mouse_position();
        if self.rows_bounds.get().contains(&position) || self.expansion.hint_contains(position) {
            return;
        }
        self.expansion.pending = None;
        if !self.expansion.by_selection && self.expansion.key.take().is_some() {
            cx.notify();
        }
    }

    fn row_at(&self, position: Point<Pixels>, cx: &App) -> Option<NodeId> {
        let tree = self.tree.as_ref()?;
        let bounds = self.rows_bounds.get();
        if !bounds.contains(&position) {
            return None;
        }
        let row_height = cx.theme().metric(Metric::RowHeight);
        let row = ((position.y - bounds.origin.y + self.scroll_top()) / row_height).floor();
        if row < 0.0 {
            return None;
        }
        tree.rows().get(row as usize).copied()
    }

    /// Top of the row at `row` in the rows' area.
    fn row_top(&self, row: usize, cx: &App) -> Pixels {
        cx.theme().metric(Metric::RowHeight) * row as f32 - self.scroll_top()
    }

    /// The row whose rest shows now, if any: cut by the tree's edge, whole
    /// in view, and pointed at (or selected while the tree has the focus).
    pub(super) fn expanded_row(&self, focused: bool, cx: &App) -> Option<NodeId> {
        let id = self.expansion.key?;
        if self.expansion.by_selection && (!focused || self.selected != Some(id)) {
            return None;
        }
        if self.context_menu.is_some() || !self.is_overflowing(id) {
            return None;
        }
        let row = self.tree.as_ref()?.row_of(id)?;
        let top = self.row_top(row, cx);
        let row_height = cx.theme().metric(Metric::RowHeight);
        (top >= px(0.0) && top + row_height <= self.rows_bounds.get().size.height).then_some(id)
    }

    /// The rest of the row the pointer is at, over the tree's edge (cut
    /// at the window's edge).
    pub(super) fn expansion_overlay(
        &self,
        focused: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        self.expansion.hint.set(None);
        let id = self.expanded_row(focused, cx)?;
        let hint = div()
            .id("tree-row-expansion")
            .debug_selector(|| "tree-row-expansion".into())
            // The editor below gets no pointer while the pointer is on it.
            .occlude()
            .on_hover(cx.listener(|this, hovered: &bool, window, cx| {
                if !hovered {
                    this.pointer_left(window, cx);
                }
            }));
        // Clicks on it are clicks on the row.
        let hint = Self::row_clicks(hint, id, cx);
        let tree = self.tree.as_ref()?;
        let node = tree.node(id);
        let end = *self.expansion.label_ends.borrow().get(&id)?;
        let theme = cx.theme();
        let rows = self.rows_bounds.get();
        let row_height = theme.metric(Metric::RowHeight);
        let border = px(HINT_BORDER);
        // It starts where the row's selection ends, so the two meet, and
        // its border runs outside the row's height: top, bottom and right,
        // none on the side that joins the row.
        let start = rows.size.width - px(WIDE_MARGIN);
        let origin = point(
            rows.left() + start,
            rows.top() + self.row_top(tree.row_of(id)?, cx) - border,
        );
        let width =
            (end - start + px(EXPANSION_PAD) + border).min(window.viewport_size().width - origin.x);
        if width <= border {
            return None;
        }
        self.expansion.hint.set(Some(Bounds::new(
            origin,
            size(width, row_height + border * 2.0),
        )));
        let selected = self.selected == Some(id);
        // The row's background as the tree draws it: the tool window's,
        // the excluded rows' over it, then the selection or the hover.
        let excluded = node
            .entry
            .excluded
            .then(|| theme.color(UiRole::NonProjectFile));
        let background = match (selected, focused) {
            (true, true) => theme.color(UiRole::Selection),
            (true, false) => theme.color(UiRole::SelectionInactive),
            _ => theme.color(UiRole::Hover),
        };
        let radius = theme.metric(Metric::CornerRadius);
        let layer = || div().absolute().top_0().left_0().size_full();
        let label = self.row_label(node, self.row_text_color(node, selected, focused, cx), cx);
        Some(
            deferred(
                anchored().position(origin).child(
                    hint.relative()
                        .w(width)
                        .h(row_height + border * 2.0)
                        .font_family(theme.ui_family())
                        .text_size(theme.ui_size())
                        .bg(theme.color(UiRole::Border))
                        .rounded_tr(radius + border)
                        .rounded_br(radius + border)
                        .child(
                            // The row's part beyond the edge, exactly as
                            // high as the row and level with it.
                            div()
                                .debug_selector(|| "tree-row-expansion-row".into())
                                .absolute()
                                .top(border)
                                .left_0()
                                .w(width - border)
                                .h(row_height)
                                .overflow_hidden()
                                .rounded_tr(radius)
                                .rounded_br(radius)
                                .bg(theme.color(UiRole::ToolWindow))
                                .children(excluded.map(|bg| layer().bg(bg)))
                                .child(layer().bg(background))
                                .child(
                                    // The row's text, continuing where the
                                    // tree cuts it.
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left(label_x(node, cx) - start)
                                        .h(row_height)
                                        .flex()
                                        .items_center()
                                        .child(label),
                                ),
                        ),
                ),
            )
            .into_any_element(),
        )
    }

    /// The row's name (and the root's location), as the row draws it.
    pub(super) fn row_label(&self, node: &Node, text: Hsla, cx: &App) -> Div {
        let mut label = div()
            .flex()
            .flex_row()
            .gap(px(LOCATION_GAP))
            .whitespace_nowrap()
            .text_color(text)
            .child(SharedString::from(node.entry.name.clone()));
        if node.depth == 0 {
            label = label.font_weight(FontWeight::SEMIBOLD);
            if let Some(project) = &self.project {
                label = label.child(
                    div()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(cx.theme().color(UiRole::TextMuted))
                        .child(project.display_root()),
                );
            }
        }
        label
    }

    /// The row whose rest shows, relative to the project (for
    /// `workbench.state`).
    pub(super) fn expansion_json(&self, focused: bool, cx: &App) -> serde_json::Value {
        let (Some(id), Some(tree), Some(project)) =
            (self.expanded_row(focused, cx), &self.tree, &self.project)
        else {
            return serde_json::Value::Null;
        };
        let path = &tree.node(id).entry.path;
        let rel = path
            .strip_prefix(project.root())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        serde_json::json!(rel)
    }

    /// The row's text ends past the tree's edge (for `workbench.state`).
    pub(super) fn overflow_json(&self, id: NodeId) -> bool {
        self.is_overflowing(id)
    }
}

/// Where the text of a row starts, from the row's left edge.
fn label_x(node: &Node, cx: &App) -> Pixels {
    px(TREE_LEFT) + cx.theme().metric(Metric::TreeIndent) * node.depth as f32 + px(ICON_X + TEXT_X)
}
