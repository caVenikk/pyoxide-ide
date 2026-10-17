//! Drawing of the Commit tool window. Sizes follow `NonModalCommitPanel`
//! (`UISpec`: insets of 12, the message border of 3, the action panel's
//! insets) and the changes tree (`ChangesTree`: chevron, check box, icon,
//! name, the parent folder in grey after a thin space).

use std::path::Path;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Corner, Entity, FocusHandle, Focusable as _,
    FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Point, Render, SharedString, StatefulInteractiveElement as _, Styled, Window, anchored,
    canvas, deferred, div, point, prelude::*, px, uniform_list,
};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, checkbox, icon_button, popup_shadow};
use ui::icons::{Icon, IconName};
use ui::menu::{ContextMenu, MenuItem, PopupOptions};
use ui::theme::ThemeExt;
use vcs::FileStatus;

use super::legend::{Chunk, SEPARATOR};
use super::tree::{Group, Inclusion, Row, file_count, origin_text, parent_text};
use super::*;
use crate::status_colors::status_role;

/// Space before the chevron, one tree level, the chevron's room, the check
/// box's room and the icon's room of a row.
const ROW_LEFT: f32 = 6.0;
const INDENT: f32 = 16.0;
const CHEVRON_ROOM: f32 = 18.0;
const CHECK_ROOM: f32 = 20.0;
const ICON_ROOM: f32 = 20.0;
/// `spaceAndThinSpace()` between a name and what follows it.
const THIN: &str = " \u{2009}";
/// Toolbar buttons and their icons.
const BUTTON: f32 = 26.0;
const ICON: f32 = 16.0;
/// The message field: its height, the gap around it (`UISpec`:
/// 12 + 2 - 3 left and right, 6 - 3 above and below) and its inner inset.
const MESSAGE_HEIGHT: f32 = 150.0;
const MESSAGE_GAP_X: f32 = 11.0;
const MESSAGE_GAP_Y: f32 = 3.0;
const MESSAGE_INSET: f32 = 6.0;
/// Insets of the status row and of the buttons (`UISpec`).
const SIDE: f32 = 12.0;
/// Height of the Commit buttons.
const COMMIT_BUTTON: f32 = 26.0;
/// The commit options popup's narrowest width
/// (`COMMIT_OPTIONS_POPUP_MINIMUM_SIZE`) and its insets (10, 14).
const OPTIONS_WIDTH: f32 = 300.0;

/// A check box that may be partly ticked (a group with some of its files
/// included): the accent fill with a dash.
fn tri_checkbox(id: impl Into<gpui::ElementId>, state: Inclusion, cx: &App) -> AnyElement {
    match state {
        Inclusion::All => checkbox(id, true, cx).into_any_element(),
        Inclusion::None => checkbox(id, false, cx).into_any_element(),
        Inclusion::Some => {
            let theme = cx.theme();
            div()
                .id(id)
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .size(px(ui::components::CHECKBOX))
                .rounded(theme.metric(Metric::CornerRadiusSmall))
                .bg(theme.color(UiRole::Accent))
                .child(
                    div()
                        .w(px(8.0))
                        .h(px(1.75))
                        .bg(theme.color(UiRole::OnAccent)),
                )
                .into_any_element()
        }
    }
}

/// A canvas that records its bounds (where a popup opens).
fn bounds_probe(cell: &std::rc::Rc<std::cell::Cell<Bounds<Pixels>>>) -> impl IntoElement {
    let cell = cell.clone();
    canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {})
        .absolute()
        .top_0()
        .left_0()
        .size_full()
}

impl CommitView {
    fn shortcut(&self, action: &dyn gpui::Action, window: &Window) -> Option<SharedString> {
        ui::keys::shortcut_in(action, &self.focus, window).map(Into::into)
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(theme.metric(Metric::ToolWindowHeaderHeight))
            .pl(px(12.0))
            .pr(px(8.0))
            .gap(px(2.0))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .whitespace_nowrap()
                    .child("Commit"),
            )
            .child(div().flex_1())
            .child(
                icon_button(
                    "commit-window-options",
                    IconName::MORE_VERTICAL,
                    px(24.0),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Options", None)),
            )
            .child(
                icon_button("commit-hide", IconName::HIDE, px(24.0), px(ICON), false, cx)
                    .debug_selector(|| "commit-hide".into())
                    .tooltip(Tooltip::text("Hide", None))
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(CommitEvent::Hide))),
            )
    }

    /// A toolbar button; a disabled one is dimmed and does nothing.
    fn tool_button(
        &self,
        id: &'static str,
        icon: IconName,
        title: &'static str,
        action: Option<Box<dyn gpui::Action>>,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let shortcut = action
            .as_ref()
            .and_then(|a| self.shortcut(a.as_ref(), window));
        let enabled = action.is_some();
        icon_button(id, icon, px(BUTTON), px(ICON), false, cx)
            .debug_selector(move || id.into())
            .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
            .tooltip(Tooltip::text(title, shortcut))
            .when_some(action, |d, action| {
                d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            })
    }

    fn render_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_changes = self
            .tree
            .selected_files()
            .iter()
            .any(|p| self.tree.group_of(p) == Some(Group::Changes));
        let has_file = self.selected_file().is_some();
        let rollback: Option<Box<dyn gpui::Action>> = has_changes.then(|| {
            Box::new(Rollback {
                paths: None,
                delete_added: None,
            }) as Box<dyn gpui::Action>
        });
        let diff: Option<Box<dyn gpui::Action>> =
            has_file.then(|| Box::new(TreeShowDiff) as Box<dyn gpui::Action>);
        let separator = div()
            .flex_none()
            .w(px(1.0))
            .h(px(18.0))
            .mx(px(4.0))
            .bg(cx.theme().color(UiRole::Divider));
        let view_options = icon_button(
            "commit-view-options",
            IconName::SHOW,
            px(BUTTON),
            px(ICON),
            self.menu.is_some(),
            cx,
        )
        .debug_selector(|| "commit-view-options".into())
        .relative()
        .tooltip(Tooltip::text("View Options", None))
        .child(bounds_probe(&self.view_options_bounds))
        .on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.open_view_options(window, cx)),
        );
        div()
            .id("commit-toolbar")
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(30.0))
            .px(px(6.0))
            .child(self.tool_button(
                "commit-refresh",
                IconName::REFRESH,
                "Refresh",
                Some(Box::new(RefreshChanges)),
                window,
                cx,
            ))
            .child(self.tool_button(
                "commit-rollback",
                IconName::ROLLBACK,
                "Rollback\u{2026}",
                rollback,
                window,
                cx,
            ))
            .child(self.tool_button("commit-diff", IconName::DIFF, "Show Diff", diff, window, cx))
            .child(self.tool_button(
                "commit-shelve",
                IconName::SHELVE,
                "Shelve Silently",
                None,
                window,
                cx,
            ))
            .child(view_options)
            .child(separator)
            .child(self.tool_button(
                "commit-locate",
                IconName::LOCATE,
                "Select Opened File",
                Some(Box::new(TreeSelectOpened)),
                window,
                cx,
            ))
            .child(self.tool_button(
                "commit-expand",
                IconName::EXPAND_ALL,
                "Expand All",
                Some(Box::new(TreeExpandAll)),
                window,
                cx,
            ))
            .child(self.tool_button(
                "commit-collapse",
                IconName::COLLAPSE_ALL,
                "Collapse All",
                Some(Box::new(TreeCollapseAll)),
                window,
                cx,
            ))
    }

    /// View Options (`ChangesView.ViewOptions`): grouping and what to show,
    /// not available yet.
    fn open_view_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = vec![
            MenuItem::header("Group By"),
            MenuItem::unavailable("Directory"),
            MenuItem::unavailable("Module"),
            MenuItem::header("Show"),
            MenuItem::unavailable("Ignored Files"),
        ];
        window.focus(&self.tree_focus);
        let menu = ContextMenu::build(items, window, cx);
        let b = self.view_options_bounds.get();
        self.open_menu(menu, point(b.left(), b.bottom() + px(2.0)), cx);
    }

    /// The drop-down of "last commit" (`AmendCommitModeDropDownLink`):
    /// the last commit, checked while amending.
    fn open_amend_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut item = MenuItem::action("last commit", AmendLast);
        if let Some(c) = &self.last_commit {
            item = item.secondary(c.subject.clone());
        }
        if self.amend {
            item = item.icon(IconName::CHECKED);
        }
        window.focus(&self.message.focus_handle(cx));
        let menu = ContextMenu::popup(vec![item], PopupOptions::default(), window, cx);
        let b = self.amend_link_bounds.get();
        self.open_menu(menu, point(b.left(), b.bottom() + px(2.0)), cx);
        if self.last_commit.is_none() {
            cx.emit(CommitEvent::LoadLastCommit);
        }
    }

    /// The context menu of the tree (a subset of `ChangesViewPopupMenu`).
    fn open_context_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let files = self.tree.selected_files();
        let changes = files
            .iter()
            .any(|p| self.tree.group_of(p) == Some(Group::Changes));
        let unversioned = files
            .iter()
            .any(|p| self.tree.group_of(p) == Some(Group::Unversioned));
        let file = self.selected_file().is_some();
        let items = vec![
            MenuItem::action(
                "Rollback\u{2026}",
                Rollback {
                    paths: None,
                    delete_added: None,
                },
            )
            .icon(IconName::ROLLBACK)
            .enabled(changes),
            MenuItem::action("Show Diff", TreeShowDiff)
                .icon(IconName::DIFF)
                .enabled(file),
            MenuItem::action("Jump to Source", TreeJumpToSource).enabled(file),
            MenuItem::separator(),
            MenuItem::action("Add to VCS", AddToVcs { paths: None }).enabled(unversioned),
            MenuItem::separator(),
            MenuItem::action("Refresh", RefreshChanges).icon(IconName::REFRESH),
        ];
        window.focus(&self.tree_focus);
        let menu = ContextMenu::build(items, window, cx);
        self.open_menu(menu, position, cx);
    }

    // ---- the tree -----------------------------------------------------------

    fn row_text_color(
        &self,
        status: Option<FileStatus>,
        selected: bool,
        focused: bool,
        cx: &App,
    ) -> Hsla {
        let theme = cx.theme();
        let role = status.and_then(status_role);
        match (role, selected && focused) {
            (Some(role), _) => theme.color(role),
            (None, true) => theme.color(UiRole::SelectionText),
            (None, false) => theme.color(UiRole::Text),
        }
    }

    fn render_row(
        &self,
        row: &Row,
        ix: usize,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let row_h = theme.metric(Metric::RowHeight);
        let radius = theme.metric(Metric::CornerRadius);
        let muted = theme.color(UiRole::TextMuted);
        let selected = self.tree.is_selected(row);
        let selection = if focused {
            theme.color(UiRole::Selection)
        } else {
            theme.color(UiRole::SelectionInactive)
        };
        let hover = theme.color(UiRole::Hover);
        let root = self.tree.root().to_path_buf();
        let (depth, selector) = match row {
            Row::Group(g) => (0usize, format!("commit-row:{}", g.id())),
            Row::File(p) => (
                1,
                format!(
                    "commit-row:{}",
                    p.strip_prefix(&root)
                        .map(|r| r.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default()
                ),
            ),
        };
        let left = px(ROW_LEFT + INDENT * depth as f32);
        let check_x = left + px(CHEVRON_ROOM);
        let inclusion = self.tree.row_inclusion(row);
        let check_row = row.clone();
        let check = div()
            .id(("commit-check", ix))
            .absolute()
            .left(check_x)
            .top((row_h - px(ui::components::CHECKBOX)) / 2.0)
            .child(tri_checkbox(("commit-checkbox", ix), inclusion, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                window.focus(&this.tree_focus);
                this.tree.select(&check_row);
                this.tree.toggle(std::slice::from_ref(&check_row));
                this.inclusion_changed(cx);
            }));
        let content: AnyElement = match row {
            Row::Group(g) => {
                let g = *g;
                let expanded = self.tree.is_expanded(g);
                let chevron = div()
                    .id(("commit-chevron", ix))
                    .absolute()
                    .left(left)
                    .top((row_h - px(ICON)) / 2.0)
                    .child(Icon::new(if expanded {
                        IconName::CHEVRON_DOWN
                    } else {
                        IconName::CHEVRON_RIGHT
                    }))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.tree.set_expanded(g, !expanded);
                        cx.notify();
                    }));
                let count = format!("{THIN}{}", file_count(self.tree.files(g).len()));
                let text_color = if selected && focused {
                    theme.color(UiRole::SelectionText)
                } else {
                    theme.color(UiRole::Text)
                };
                div()
                    .child(chevron)
                    .child(
                        div()
                            .absolute()
                            .left(check_x + px(CHECK_ROOM))
                            .top_0()
                            .h(row_h)
                            .flex()
                            .flex_row()
                            .items_center()
                            .whitespace_nowrap()
                            .child(
                                div()
                                    .text_color(text_color)
                                    .when(g == Group::Changes, |d| d.font_weight(FontWeight::BOLD))
                                    .child(g.title()),
                            )
                            .child(div().text_color(muted).child(count)),
                    )
                    .into_any_element()
            }
            Row::File(path) => {
                let change = self.tree.change(path);
                let status = change.map(|c| c.status).unwrap_or(FileStatus::Unversioned);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let origin = change.and_then(|c| origin_text(c, &root));
                let parent = parent_text(path, &root);
                let icon = (self.file_icon)(path);
                let text_color = if selected && focused {
                    theme.color(UiRole::SelectionText)
                } else {
                    theme.color(UiRole::Text)
                };
                div()
                    .child(
                        div()
                            .absolute()
                            .left(check_x + px(CHECK_ROOM))
                            .top((row_h - px(ICON)) / 2.0)
                            .child(Icon::new(icon)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(check_x + px(CHECK_ROOM + ICON_ROOM))
                            .top_0()
                            .h(row_h)
                            .flex()
                            .flex_row()
                            .items_center()
                            .whitespace_nowrap()
                            .child(
                                div()
                                    .text_color(self.row_text_color(
                                        Some(status),
                                        selected,
                                        focused,
                                        cx,
                                    ))
                                    .child(name),
                            )
                            .children(
                                origin.map(|o| {
                                    div().text_color(text_color).child(format!("{THIN}{o}"))
                                }),
                            )
                            .when(!parent.is_empty(), |d| {
                                d.child(div().text_color(muted).child(format!("{THIN}{parent}")))
                            }),
                    )
                    .into_any_element()
            }
        };
        let click_row = row.clone();
        let menu_row = row.clone();
        div()
            .id(("commit-row", ix))
            .debug_selector(move || selector.clone())
            .relative()
            .w_full()
            .h(row_h)
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(4.0))
                    .right(px(4.0))
                    .rounded(radius)
                    .when(selected, |d| d.bg(selection))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover))),
            )
            .child(content)
            .child(check)
            .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                window.focus(&this.tree_focus);
                let m = e.modifiers();
                if m.shift {
                    this.tree.select_range(&click_row);
                } else if m.secondary() {
                    this.tree.toggle_selected(&click_row);
                } else {
                    this.tree.select(&click_row);
                }
                if e.click_count() >= 2 && !m.shift && !m.secondary() {
                    this.tree_open(&TreeOpen, window, cx);
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if !this.tree.is_selected(&menu_row) {
                        this.tree.select(&menu_row);
                    }
                    this.open_context_menu(e.position, window, cx);
                }),
            )
            .into_any_element()
    }

    fn render_tree(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let focused = self.tree_focus.is_focused(window);
        let el = div()
            .id("commit-tree")
            .debug_selector(|| "commit-tree".into())
            .key_context(TREE_KEY_CONTEXT)
            .track_focus(&self.tree_focus)
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(80.0))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, _| window.focus(&this.tree_focus)),
            );
        match self.state {
            RepoState::None => {
                let init = crate::Init { path: None };
                return el
                    .items_center()
                    .justify_center()
                    .text_color(theme.color(UiRole::TextMuted))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .justify_center()
                            .gap(px(4.0))
                            .child("To commit changes,")
                            .child(
                                ui::link::link(
                                    "commit-create-repository",
                                    "Create Git repository\u{2026}",
                                    cx,
                                )
                                .on_click(move |_, window, cx| {
                                    window.dispatch_action(Box::new(init.clone()), cx)
                                }),
                            ),
                    )
                    .into_any_element();
            }
            RepoState::Loading => return el.into_any_element(),
            RepoState::Ready => {}
        }
        if self.tree.is_empty() {
            return el
                .items_center()
                .justify_center()
                .text_color(theme.color(UiRole::TextMuted))
                .child(
                    div()
                        .debug_selector(|| "commit-empty".into())
                        .child(EMPTY_TREE),
                )
                .into_any_element();
        }
        // The rows of this frame, made once for the visible range.
        let rows = std::rc::Rc::new(self.tree.rows());
        let count = rows.len();
        let list = uniform_list(
            "commit-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|ix| rows.get(ix).map(|r| this.render_row(r, ix, focused, cx)))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(self.scroll.clone())
        .flex_1()
        .overflow_x_hidden();
        el.child(list).into_any_element()
    }

    // ---- the commit panel ---------------------------------------------------

    /// The width of a text in the UI font.
    fn text_width(&self, text: &str, window: &Window, cx: &App) -> Pixels {
        let theme = cx.theme();
        let run = gpui::TextRun {
            len: text.len(),
            font: gpui::font(theme.ui_family()),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(
                SharedString::from(text.to_owned()),
                theme.ui_size(),
                &[run],
                None,
            )
            .width
    }

    fn render_status_row(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let legend = self.legend();
        // The legend shortens when the full one does not fit beside the
        // amend part (`CommitStatusPanel.adjustLegendToFitPanel`).
        let amend_width = px(10.0 + 16.0 + 6.0 + 4.0 + 16.0 + 6.0 + 22.0 + SIDE + 16.0)
            + self.text_width("Amend", window, cx)
            + self.text_width("last commit", window, cx);
        let full = legend.text(false);
        let compact = self.width.get() - amend_width < self.text_width(&full, window, cx);
        let chunks: Vec<Chunk> = legend.chunks(compact);
        let mut legend_el = div()
            .id("commit-legend")
            .debug_selector(|| "commit-legend".into())
            .flex()
            .flex_row()
            .flex_none()
            .whitespace_nowrap();
        for (i, chunk) in chunks.into_iter().enumerate() {
            if i > 0 {
                legend_el = legend_el.child(SEPARATOR);
            }
            let color =
                status_role(chunk.status).map_or(theme.color(UiRole::Text), |r| theme.color(r));
            legend_el = legend_el.child(div().text_color(color).child(chunk.text));
        }
        let amend = self.amend;
        let enabled = self.state == RepoState::Ready;
        let toggle = div()
            .id("commit-amend")
            .debug_selector(|| "commit-amend".into())
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .when(!enabled, |d| d.opacity(ui::components::DISABLED_ALPHA))
            .child(checkbox("commit-amend-check", amend, cx))
            .child("Amend")
            .tooltip(Tooltip::text(
                "Amend Commit",
                self.shortcut(&ToggleAmend, window),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.toggle_amend(&ToggleAmend, window, cx)
            }));
        let link = ui::link::link("commit-amend-target", "last commit", cx)
            .debug_selector(|| "commit-amend-target".into())
            .relative()
            .child(Icon::new(IconName::CHEVRON_DOWN).size(px(ICON)))
            .child(bounds_probe(&self.amend_link_bounds))
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.open_amend_menu(window, cx)),
            );
        let history = icon_button(
            "commit-history",
            IconName::HISTORY,
            px(22.0),
            px(ICON),
            false,
            cx,
        )
        .opacity(ui::components::DISABLED_ALPHA)
        .tooltip(Tooltip::text("Commit Message History", None));
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(30.0))
            .pl(px(10.0))
            .pr(px(SIDE))
            .gap(px(4.0))
            .child(toggle)
            .child(link)
            .child(history)
            .child(div().flex_1().min_w(px(8.0)))
            .child(legend_el)
    }

    fn commit_button(
        &self,
        id: &'static str,
        label: &'static str,
        primary: bool,
        enabled: bool,
        action: Box<dyn gpui::Action>,
        cx: &App,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let hover = theme.color(UiRole::Hover);
        // The first button keeps its width; the second gives way (its
        // text cut with an ellipsis) so that the gear stays in a narrow
        // tool window.
        let first = id == "commit-button";
        div()
            .id(id)
            .debug_selector(move || id.into())
            .flex()
            .when(first, |d| d.flex_none().min_w(px(72.0)))
            .when(!first, |d| d.flex_shrink().min_w(px(0.0)).overflow_hidden())
            .items_center()
            .justify_center()
            .h(px(COMMIT_BUTTON))
            .px(px(14.0))
            .rounded(ui::form::field_radius(cx))
            .whitespace_nowrap()
            .map(|d| {
                if primary && enabled {
                    d.bg(theme.color(UiRole::Accent))
                        .text_color(theme.color(UiRole::OnAccent))
                } else {
                    d.border_1()
                        .border_color(theme.color(UiRole::BorderInput))
                        .when(enabled, |d| d.hover(move |s| s.bg(hover)))
                }
            })
            .when(!enabled, |d| {
                d.text_color(theme.color(UiRole::TextDisabled))
            })
            .child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(label),
            )
            .when(enabled, |d| {
                d.on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
            })
    }

    fn render_commit_panel(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let message_focused = self.message.focus_handle(cx).is_focused(window);
        let ready = self.state == RepoState::Ready && !self.busy;
        let default = ready && self.focus.contains_focused(window, cx);
        let border = theme.color(UiRole::BorderInput);
        let ring = theme.color(UiRole::Focus);
        let radius = theme.metric(Metric::CornerRadius) * 1.5;
        let message = div()
            .id("commit-message")
            .debug_selector(|| "commit-message".into())
            .key_context(MESSAGE_KEY_CONTEXT)
            .flex_none()
            .h(px(MESSAGE_HEIGHT))
            .mx(px(MESSAGE_GAP_X))
            .my(px(MESSAGE_GAP_Y))
            .rounded(radius)
            .border_1()
            .border_color(if message_focused { ring } else { border })
            .bg(theme.color(UiRole::EditorBackground))
            .p(px(1.0))
            .pl(px(MESSAGE_INSET))
            .overflow_hidden()
            .child(div().size_full().child(self.message.clone()));
        let problem = self.problem.as_ref().map(|p| {
            div()
                .debug_selector(|| "commit-problem".into())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .px(px(SIDE))
                .py(px(4.0))
                .text_color(theme.color(UiRole::Danger))
                .child(Icon::new(IconName::ERROR))
                .child(p.text().to_owned())
        });
        let gear = icon_button(
            "commit-options",
            IconName::SETTINGS_SMALL,
            px(24.0),
            px(ICON),
            self.options.open,
            cx,
        )
        .debug_selector(|| "commit-options".into())
        .relative()
        .tooltip(Tooltip::text(
            "Show Commit Options",
            self.shortcut(&ShowCommitOptions, window),
        ))
        .child(bounds_probe(&self.gear_bounds))
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.show_options(&ShowCommitOptions, window, cx)
        }));
        let buttons = div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .pt(px(3.0))
            .pb(px(8.0))
            .pl(px(SIDE - 3.0))
            .pr(px(SIDE))
            .gap(px(6.0))
            .child(self.commit_button(
                "commit-button",
                action_name(self.amend),
                default,
                ready,
                Box::new(RunCommit { message: None }),
                cx,
            ))
            .child(self.commit_button(
                "commit-push-button",
                push_action_name(self.amend),
                false,
                ready,
                Box::new(RunCommitAndPush { message: None }),
                cx,
            ))
            .children(
                self.busy
                    .then(|| ui::components::spinner("commit-busy", cx)),
            )
            .child(div().flex_1())
            .child(gear);
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(self.render_status_row(window, cx))
            .child(message)
            .children(problem)
            .child(buttons)
    }

    /// The commit options popup above the gear (`showPopupAbove`).
    fn render_options(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.options.open {
            return None;
        }
        let theme = cx.theme();
        let gear = self.gear_bounds.get();
        let size = window.viewport_size();
        let author = editor::field::frame(&self.options.author, true, false, window, cx).flex_1();
        let sign_off = ui::form::check_row(
            "commit-sign-off",
            "commit-sign-off-check",
            self.options.sign_off,
            "Sign-off commit",
            cx,
        )
        .debug_selector(|| "commit-sign-off".into())
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle_sign_off(&ToggleSignOff, window, cx)
        }));
        let hooks = self.options.hooks.then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(ui::form::group_header("Commit Checks", cx))
                .child(
                    ui::form::check_row(
                        "commit-run-hooks",
                        "commit-run-hooks-check",
                        self.options.run_hooks,
                        "Run Git hooks",
                        cx,
                    )
                    .debug_selector(|| "commit-run-hooks".into())
                    .tooltip(Tooltip::text(
                        "If unchecked, Git hooks will be skipped with the '--no-verify' option for the upcoming commit",
                        None,
                    ))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_run_hooks(&ToggleRunHooks, window, cx)
                    })),
                )
        });
        let popup = div()
            .id("commit-options-popup")
            .debug_selector(|| "commit-options-popup".into())
            .key_context(OPTIONS_KEY_CONTEXT)
            .track_focus(&self.options_focus)
            .occlude()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .min_w(px(OPTIONS_WIDTH))
            .py(px(10.0))
            .px(px(14.0))
            .rounded(theme.metric(Metric::CornerRadius) * 2.0)
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(popup_shadow())
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(ui::form::group_header("Git", cx))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().flex_none().child("Author:"))
                    .child(author),
            )
            .child(sign_off)
            .children(hooks);
        let view = cx.entity();
        Some(
            deferred(
                anchored().position(point(px(0.0), px(0.0))).child(
                    div()
                        .id("commit-options-backdrop")
                        .occlude()
                        .w(size.width)
                        .h(size.height)
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            view.update(cx, |v, cx| v.close_options(window, cx))
                        })
                        .child(
                            anchored()
                                .position(point(gear.left(), gear.top() - px(3.0)))
                                .anchor(Corner::BottomLeft)
                                .snap_to_window_with_margin(px(4.0))
                                .child(popup),
                        ),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

/// The view's focus handles for the workbench (activation, the tool
/// window's focus).
impl CommitView {
    pub fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    /// Whether `path` is listed.
    pub fn lists(&self, path: &Path) -> bool {
        self.tree.group_of(path).is_some()
    }

    pub fn message_entity(&self) -> &Entity<Editor> {
        &self.message
    }
}

impl Render for CommitView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self.width.clone();
        let view = cx.entity_id();
        let menu = self
            .menu
            .as_ref()
            .map(|m| ui::menu::overlay(&m.menu, m.position, window));
        let options = self.render_options(window, cx);
        let theme = cx.theme();
        let el = div()
            .id("commit-view")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(theme.color(UiRole::Text))
            .child(
                canvas(
                    move |b, _, cx| {
                        if width.replace(b.size.width) != b.size.width {
                            cx.notify(view);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        let el = Self::bind_actions(el, cx);
        let el = el.child(self.render_header(cx));
        let el = if self.state == RepoState::None {
            el.child(self.render_tree(window, cx))
        } else {
            el.child(self.render_toolbar(window, cx))
                .child(self.render_tree(window, cx))
                .child(self.render_commit_panel(window, cx))
        };
        el.children(menu).children(options)
    }
}
