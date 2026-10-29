//! Dragging an editor tab (the reference's `DragHelper`, the tab's
//! `DragOutDelegate` and `DockableEditorTabbedContainer`): within its tab
//! strip the tab changes place; over another group's strip it takes the
//! slot that opens there; over the middle of an editor it goes to that
//! group; over the edge of an editor it goes into a new split on that
//! side. Holding Control copies the file instead of moving the tab
//! (`UIUtil.isControlKeyDown`): the other group shows a second view of it.
//!
//! The tab is dragged as a floating chip with its icon and name; the
//! place it would go to is shown as a slot in the strip or as the half
//! (or the whole) of the editor, in the drag and drop area colour.

use gpui::{
    AnyElement, Context, DragMoveEvent, EntityId, InteractiveElement, IntoElement, ParentElement,
    Pixels, Render, SharedString, Styled, Window, div, px, relative,
};
use themes::{Metric, UiRole};
use ui::components::popup_shadow;
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::Workbench;
use crate::split_tree::{GroupId, Node};

/// The share of an editor's width and height that the drop areas of its
/// edges take (the registry key `ide.tabbedPane.dragToSplitRatio`).
const SPLIT_RATIO: f32 = 0.2;

/// Width of a slot when the dragged tab's label was not measured yet.
const SLOT_WIDTH: f32 = 100.0;

/// A tab being dragged.
#[derive(Clone)]
pub(crate) struct DraggedTab {
    pub(super) key: EntityId,
    pub(super) title: SharedString,
    pub(super) icon: IconName,
    /// The width of the tab's label: that of the slot.
    pub(super) width: Pixels,
}

impl DraggedTab {
    pub(in crate::workbench) fn new(
        key: EntityId,
        title: SharedString,
        icon: IconName,
        width: Pixels,
    ) -> Self {
        Self {
            key,
            title,
            icon,
            width,
        }
    }
}

/// The floating image of the dragged tab.
pub(crate) struct TabChip {
    title: SharedString,
    icon: IconName,
}

impl TabChip {
    pub(crate) fn new(dragged: &DraggedTab) -> Self {
        Self {
            title: dragged.title.clone(),
            icon: dragged.icon,
        }
    }
}

impl Render for TabChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        // The image is drawn outside the window's root: the UI font is set
        // here.
        div()
            .debug_selector(|| "tab-drag-image".into())
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .h(theme.metric(Metric::TabSelectionHeight))
            .px(px(8.0))
            .rounded(theme.metric(Metric::CornerRadius))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(popup_shadow())
            .whitespace_nowrap()
            .child(Icon::new(self.icon))
            .child(self.title.clone())
    }
}

/// A side of an editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    /// The new group is above or below (a vertical splitter) rather than
    /// beside.
    fn vertical(self) -> bool {
        matches!(self, Side::Top | Side::Bottom)
    }

    /// The new group comes first in its splitter.
    fn first(self) -> bool {
        matches!(self, Side::Left | Side::Top)
    }
}

/// Where a dragged tab would be put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DropPlace {
    /// Into the group's strip before its `n`-th tab (the dragged tab not
    /// counted), after the last one when `n` is the tab count.
    Strip(usize),
    /// Onto the group's editor: its middle (`None`) or a side.
    Area(Option<Side>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DropTarget {
    pub(super) group: GroupId,
    pub(super) place: DropPlace,
}

/// Whether `p` is inside the convex polygon `poly`.
fn inside(poly: &[(f32, f32)], p: (f32, f32)) -> bool {
    let mut sign = 0.0f32;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let cross = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        if cross != 0.0 {
            if sign != 0.0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    true
}

/// `TabsUtil.getDropSideFor`: the side of an editor of size `w` by `h`
/// (the tab strip included) that the point `(x, y)` is in: four
/// trapezoids along the edges, each as deep as [`SPLIT_RATIO`] of the
/// size, meeting at slanted lines from the corners; `None` between them.
pub(super) fn drop_side(x: f32, y: f32, w: f32, h: f32) -> Option<Side> {
    let r = SPLIT_RATIO;
    let p = (x, y);
    let right = [
        (w, 0.0),
        (w * (1.0 - r), h * r),
        (w * (1.0 - r), h * (1.0 - r)),
        (w, h),
    ];
    let left = [(0.0, 0.0), (w * r, h * r), (w * r, h * (1.0 - r)), (0.0, h)];
    let bottom = [
        (0.0, h),
        (w * r, h * (1.0 - r)),
        (w * (1.0 - r), h * (1.0 - r)),
        (w, h),
    ];
    let top = [(0.0, 0.0), (w, 0.0), (w * (1.0 - r), h * r), (w * r, h * r)];
    if inside(&right, p) {
        Some(Side::Right)
    } else if inside(&left, p) {
        Some(Side::Left)
    } else if inside(&bottom, p) {
        Some(Side::Bottom)
    } else if inside(&top, p) {
        Some(Side::Top)
    } else {
        None
    }
}

/// The slot a pointer at `x` is over in a strip whose tabs (the dragged
/// one left out) have the given centres: before the first tab whose centre
/// is right of the pointer.
fn strip_slot(centres: &[f32], x: f32) -> usize {
    centres.iter().take_while(|&&c| c <= x).count()
}

/// Whether a tab dropped at slot `at` among tabs with the pinned flags
/// `others` is pinned (`DockableEditorTabbedContainer.add`): a pinned tab
/// stays pinned at the start and next to pinned tabs, an unpinned one
/// becomes pinned before a pinned tab.
fn pinned_at(was_pinned: bool, others: &[bool], at: usize) -> bool {
    let at_pinned = others.get(at).copied();
    let after_pinned = at > 0 && others[at - 1];
    if was_pinned {
        at == 0 || at_pinned == Some(true) || after_pinned
    } else {
        at_pinned.unwrap_or(false)
    }
}

impl Workbench {
    /// The pointer moved during a tab drag: where the tab would go in
    /// `group`, if the pointer is over it.
    pub(super) fn tab_drag_over(
        &mut self,
        group: GroupId,
        e: &DragMoveEvent<DraggedTab>,
        cx: &mut Context<Self>,
    ) {
        let dragged = e.drag(cx);
        let key = dragged.key;
        if self.splits.dragging.map(|(k, _)| k) != Some(key) {
            self.splits.dragging = Some((key, dragged.width));
            cx.notify();
        }
        let mine = self.splits.drop.is_some_and(|d| d.group == group);
        if !e.bounds.contains(&e.event.position) {
            if mine {
                self.splits.drop = None;
                cx.notify();
            }
            return;
        }
        let header = cx.theme().metric(Metric::TabHeight);
        let at = e.event.position - e.bounds.origin;
        let tabs = self.group_tabs(group);
        let place = if tabs.is_empty() {
            DropPlace::Area(None)
        } else if at.y < header {
            let bounds = self.tab_bounds.borrow();
            let centres: Vec<f32> = tabs
                .iter()
                .filter(|&&i| self.tabs[i].key() != key)
                .filter_map(|&i| bounds.get(i).map(|b| f32::from(b.center().x)))
                .collect();
            DropPlace::Strip(strip_slot(&centres, f32::from(e.event.position.x)))
        } else {
            DropPlace::Area(drop_side(
                f32::from(at.x),
                f32::from(at.y),
                f32::from(e.bounds.size.width),
                f32::from(e.bounds.size.height),
            ))
        };
        let target = Some(DropTarget { group, place });
        if self.splits.drop != target {
            self.splits.drop = target;
            cx.notify();
        }
    }

    /// The tab is dropped on `group`: at the place the last move found.
    pub(super) fn drop_tab(
        &mut self,
        group: GroupId,
        dragged: &DraggedTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.splits.drop.take().filter(|d| d.group == group);
        self.splits.dragging = None;
        cx.notify();
        let (Some(target), Some(source)) = (
            target,
            self.tabs.iter().position(|t| t.key() == dragged.key),
        ) else {
            return;
        };
        // Only a file can be shown in two places.
        let copy = window.modifiers().control && self.tabs[source].path().is_some();
        match target.place {
            DropPlace::Strip(at) => self.drop_in_group(source, group, Some(at), copy, window, cx),
            DropPlace::Area(None) => self.drop_in_group(source, group, None, copy, window, cx),
            DropPlace::Area(Some(side)) => self.drop_on_side(source, group, side, copy, window, cx),
        }
    }

    /// Puts tab `index` into `group` before the `at`-th of the group's
    /// other tabs (after them all when `at` is their count).
    fn place_tab(&mut self, index: usize, group: GroupId, at: usize) {
        let others: Vec<usize> = self
            .group_tabs(group)
            .into_iter()
            .filter(|&i| i != index)
            .collect();
        let mut order: Vec<usize> = (0..self.tabs.len()).filter(|&i| i != index).collect();
        let slot = match others.get(at) {
            Some(&before) => order.iter().position(|&i| i == before),
            None => others
                .last()
                .and_then(|&last| order.iter().position(|&i| i == last))
                .map(|p| p + 1),
        };
        order.insert(slot.unwrap_or(order.len()), index);
        self.permute_tabs(order);
    }

    /// The pinned flag the tab gets when it is placed at `at` in `group`.
    fn pinned_for_drop(&self, index: usize, group: GroupId, at: usize) -> bool {
        let others: Vec<bool> = self
            .group_tabs(group)
            .into_iter()
            .filter(|&i| i != index)
            .map(|i| self.tabs[i].pinned)
            .collect();
        pinned_at(self.tabs[index].pinned, &others, at)
    }

    /// The tab was dropped in a strip (`at`) or on the editor of `group`:
    /// it changes place in its group, or moves to the group (`copy`: its
    /// file shows there too). A file the group shows already is not opened
    /// twice: its tab is selected.
    fn drop_in_group(
        &mut self,
        source: usize,
        group: GroupId,
        at: Option<usize>,
        copy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = self.tabs[source].group;
        let key = self.tabs[source].key();
        let activate =
            |this: &mut Self, key: EntityId, window: &mut Window, cx: &mut Context<Self>| {
                if let Some(i) = this.tabs.iter().position(|t| t.key() == key) {
                    this.activate(i, window, cx);
                }
            };
        if from == group {
            // Onto its own editor nothing changes; in the strip it moves.
            if let (Some(at), false) = (at, copy) {
                let pinned = self.pinned_for_drop(source, group, at);
                self.tabs[source].pinned = pinned;
                self.place_tab(source, group, at);
            }
            activate(self, key, window, cx);
            return;
        }
        let existing = self.tabs[source].path().and_then(|path| {
            self.group_tabs(group)
                .into_iter()
                .find(|&i| self.tabs[i].is_file(path))
        });
        if let Some(existing) = existing {
            let existing = self.tabs[existing].key();
            if !copy {
                self.close_tab(source, window, cx);
            }
            activate(self, existing, window, cx);
            return;
        }
        let end = self.group_tabs(group).len();
        let at = at.unwrap_or(end);
        if copy {
            if let Some(mirror) = self.open_mirror(source, group, window, cx) {
                let key = self.tabs[mirror].key();
                let pinned = self.pinned_for_drop(mirror, group, at);
                self.tabs[mirror].pinned = pinned;
                self.place_tab(mirror, group, at);
                activate(self, key, window, cx);
            }
            return;
        }
        let pinned = self.pinned_for_drop(source, group, at);
        self.hand_over_selection(from, source);
        self.tabs[source].group = group;
        self.tabs[source].pinned = pinned;
        self.place_tab(source, group, at);
        if self.group_tabs(from).is_empty() {
            self.remove_group(from);
        }
        activate(self, key, window, cx);
    }

    /// The tab was dropped on a side of `group`'s editor: a new group
    /// there (`EditorWindow.split` with `forceSplit`) takes the tab or,
    /// with `copy`, a second view of its file.
    fn drop_on_side(
        &mut self,
        source: usize,
        group: GroupId,
        side: Side,
        copy: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let from = self.tabs[source].group;
        let key = self.tabs[source].key();
        // The only tab of a group dropped on the side of its own group
        // would leave nothing behind.
        if !copy && from == group && self.group_tabs(group).len() < 2 {
            return;
        }
        let new = self.splits.new_group();
        let mut root = std::mem::replace(&mut self.splits.root, Node::Leaf(0));
        root.split(group, side.vertical(), new, side.first());
        self.splits.root = root;
        if copy {
            if self.open_mirror(source, new, window, cx).is_none() {
                let root = std::mem::replace(&mut self.splits.root, Node::Leaf(0));
                self.splits.root = root.without(new).unwrap_or(Node::Leaf(0));
                return;
            }
        } else {
            self.hand_over_selection(from, source);
            self.tabs[source].group = new;
            self.tabs[source].pinned = false;
            if self.group_tabs(from).is_empty() {
                self.remove_group(from);
            }
        }
        // The moved tab keeps its place in the list; a copy is the last.
        let shown = if copy {
            self.tabs.len().checked_sub(1)
        } else {
            self.tabs.iter().position(|t| t.key() == key)
        };
        if let Some(shown) = shown {
            self.activate(shown, window, cx);
        }
    }

    /// What the drag of a tab draws over `group`: the drop area on its
    /// editor (`MyDropAreaPainter`: the half of the editor for a side, the
    /// whole of it for the middle, below the strip).
    pub(super) fn drop_area(&self, group: GroupId, cx: &Context<Self>) -> Option<AnyElement> {
        let target = self.drag_drop_target(cx).filter(|d| d.group == group)?;
        let DropPlace::Area(side) = target.place else {
            return None;
        };
        let theme = cx.theme();
        let half = div()
            .debug_selector(|| "tab-drop-area".into())
            .absolute()
            .bg(theme.color(UiRole::DragAndDropArea));
        let half = match side {
            None => half.size_full(),
            Some(Side::Left) => half.top_0().bottom_0().left_0().w(relative(0.5)),
            Some(Side::Right) => half.top_0().bottom_0().right_0().w(relative(0.5)),
            Some(Side::Top) => half.left_0().right_0().top_0().h(relative(0.5)),
            Some(Side::Bottom) => half.left_0().right_0().bottom_0().h(relative(0.5)),
        };
        Some(
            div()
                .absolute()
                .top(theme.metric(Metric::TabHeight))
                .left_0()
                .right_0()
                .bottom_0()
                .child(half)
                .into_any_element(),
        )
    }

    /// The slot that opens in `group`'s strip for the dragged tab, and the
    /// place of the tab among the strip's tabs (the dragged one left out).
    pub(super) fn drop_slot(
        &self,
        group: GroupId,
        cx: &Context<Self>,
    ) -> Option<(usize, AnyElement)> {
        let target = self.drag_drop_target(cx).filter(|d| d.group == group)?;
        let DropPlace::Strip(at) = target.place else {
            return None;
        };
        let theme = cx.theme();
        let width = self
            .splits
            .dragging
            .map(|(_, w)| w)
            .filter(|w| *w > px(0.0))
            .unwrap_or(px(SLOT_WIDTH));
        let slot = div()
            .flex_none()
            .px(px(super::super::TAB_H_OFFSET))
            .child(
                div()
                    .debug_selector(|| "tab-drop-slot".into())
                    .w(width)
                    .h(theme.metric(Metric::TabSelectionHeight))
                    .rounded(theme.metric(Metric::ButtonRadius))
                    .bg(theme.color(UiRole::DragAndDropArea)),
            )
            .into_any_element();
        Some((at, slot))
    }

    /// The drop target while a tab is dragged.
    fn drag_drop_target(&self, cx: &gpui::App) -> Option<DropTarget> {
        if self.splits.dragging.is_none() || !cx.has_active_drag() {
            return None;
        }
        self.splits.drop
    }

    /// A tab is being dragged: where it would go (`state`), else `null`.
    pub(super) fn tab_drag_json(&self, cx: &gpui::App) -> serde_json::Value {
        if self.splits.dragging.is_none() || !cx.has_active_drag() {
            return serde_json::Value::Null;
        }
        let place = self.splits.drop.map(|d| {
            let place = match d.place {
                DropPlace::Strip(at) => format!("strip {at}"),
                DropPlace::Area(None) => "area".to_owned(),
                DropPlace::Area(Some(side)) => format!("area {side:?}").to_lowercase(),
            };
            serde_json::json!({ "group": d.group, "place": place })
        });
        serde_json::json!({ "drop": place })
    }

    /// The tab being dragged now, if any; it is not drawn in its strip.
    pub(super) fn dragged_tab(&self, cx: &Context<Self>) -> Option<EntityId> {
        self.splits
            .dragging
            .filter(|_| cx.has_active_drag())
            .map(|(key, _)| key)
    }

    /// The button went up: a drag that was not dropped anywhere is over.
    pub(super) fn tab_drag_ended(&mut self, cx: &mut Context<Self>) {
        if self.splits.dragging.take().is_some() {
            self.splits.drop = None;
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_middle_of_an_editor_has_no_side_and_the_edges_have_theirs() {
        let (w, h) = (1000.0, 600.0);
        assert_eq!(drop_side(500.0, 300.0, w, h), None);
        assert_eq!(drop_side(980.0, 300.0, w, h), Some(Side::Right));
        assert_eq!(drop_side(20.0, 300.0, w, h), Some(Side::Left));
        assert_eq!(drop_side(500.0, 20.0, w, h), Some(Side::Top));
        assert_eq!(drop_side(500.0, 590.0, w, h), Some(Side::Bottom));
        // The slanted lines from the corners: near the top-left corner the
        // point is below the line to (0.2 w, 0.2 h), so it is left.
        assert_eq!(drop_side(10.0, 60.0, w, h), Some(Side::Left));
        assert_eq!(drop_side(150.0, 5.0, w, h), Some(Side::Top));
    }

    #[test]
    fn a_pointer_picks_the_slot_before_the_first_tab_right_of_it() {
        let centres = [50.0, 150.0, 250.0];
        assert_eq!(strip_slot(&centres, 10.0), 0);
        assert_eq!(strip_slot(&centres, 100.0), 1);
        assert_eq!(strip_slot(&centres, 200.0), 2);
        assert_eq!(strip_slot(&centres, 900.0), 3);
        assert_eq!(strip_slot(&[], 10.0), 0);
    }

    #[test]
    fn pinned_tabs_stay_pinned_among_pinned_tabs_only() {
        // Others: pinned, pinned, unpinned.
        let others = [true, true, false];
        assert!(pinned_at(true, &others, 0));
        assert!(pinned_at(true, &others, 1));
        assert!(pinned_at(true, &others, 2), "after a pinned tab");
        assert!(!pinned_at(true, &others, 3), "after the unpinned one");
        assert!(pinned_at(false, &others, 1), "before a pinned tab");
        assert!(!pinned_at(false, &others, 2));
        assert!(!pinned_at(false, &others, 3));
    }
}
