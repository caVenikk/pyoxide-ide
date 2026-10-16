//! The Notifications tool window of the right stripe (the reference's
//! `NotificationsToolWindowFactory` and `NotificationsPanel`, New UI):
//! the empty text while there is nothing, else the "Timeline" group with
//! its "Clear all" link and the notifications, the newest first, each
//! with its kind's icon, title, time, content and action links.
//!
//! Not yet: the Suggestions group, search, "Mark all as read", the unread
//! highlight and the per-notification menu.

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, IntoElement,
    Render, Subscription, Window, div, prelude::*, px,
};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, icon_button};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use super::{Notifications, TimelineEntry};
use crate::commands::HideToolWindow;
use crate::tool_windows::ToolWindowView;

/// The tool window's id (`NotificationsToolWindowFactory.ID`).
pub const ID: &str = "notifications";

/// The empty text (`notifications.toolwindow.empty.text.first.line` and
/// `.second.line` of `IdeBundle`).
pub const EMPTY_LINES: [&str; 2] = ["Suggestions, events,", "and errors will appear here"];
/// The group's title and its link (`notifications.toolwindow.timeline`,
/// `.timeline.clear.all`).
pub const TIMELINE: &str = "Timeline";
pub const CLEAR_ALL: &str = "Clear all";

/// `NotificationGroupComponent`: 8 above the group, its title 18 from the
/// left and the link 20 from the right, 8 between title and list; the
/// list 8 from the left and 10 from the right, 10 between notifications.
const GROUP_TOP: f32 = 8.0;
const TITLE_LEFT: f32 = 18.0;
const LINK_RIGHT: f32 = 20.0;
const TITLE_GAP: f32 = 8.0;
const LIST_LEFT: f32 = 8.0;
const LIST_RIGHT: f32 = 10.0;
const LIST_GAP: f32 = 10.0;
/// `NotificationComponent`: 10 around it (none on the right), 7 between
/// the icon and the text, 8 between title, content and links, 10 right
/// of the text and of the time, 16 between links.
const ENTRY_INSET: f32 = 10.0;
const ICON_GAP: f32 = 7.0;
const LINE_GAP: f32 = 8.0;
const TEXT_RIGHT: f32 = 10.0;
const ACTION_GAP: f32 = 16.0;
const ICON: f32 = 16.0;
const BUTTON: f32 = 24.0;

/// The tool window's view: it lists the window's [`Notifications`].
pub struct NotificationsToolWindow {
    notifications: Entity<Notifications>,
    focus: FocusHandle,
    _observe: Subscription,
}

impl NotificationsToolWindow {
    pub fn new(notifications: Entity<Notifications>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&notifications, |_, _, cx| cx.notify());
        Self {
            notifications,
            focus: cx.focus_handle(),
            _observe: observe,
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let focus = self.focus.clone();
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
                    .child("Notifications"),
            )
            .child(div().flex_1())
            .child(
                icon_button(
                    "notifications-options",
                    IconName::MORE_VERTICAL,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("Options", None)),
            )
            .child(
                icon_button(
                    "notifications-hide",
                    IconName::HIDE,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .debug_selector(|| "notifications-hide".into())
                .tooltip(Tooltip::text("Hide", None))
                // Hide Active Tool Window hides the one with the focus.
                .on_click(move |_: &ClickEvent, window, cx| {
                    window.focus(&focus);
                    window.dispatch_action(Box::new(HideToolWindow), cx);
                }),
            )
    }

    /// The empty text, centred (`JBPanelWithEmptyText`).
    fn render_empty(&self, cx: &App) -> AnyElement {
        div()
            .id("notifications-empty")
            .debug_selector(|| "notifications-empty".into())
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .text_color(cx.theme().color(UiRole::TextMuted))
            .children(EMPTY_LINES)
            .into_any_element()
    }

    fn render_timeline(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let small = theme.ui_size() - px(1.0);
        let muted = theme.color(UiRole::TextMuted);
        let notifications = self.notifications.clone();
        let clear = ui::link::link("notifications-clear-all", CLEAR_ALL, cx)
            .debug_selector(|| "notifications-clear-all".into())
            .text_size(small)
            .on_click(move |_, _, cx| {
                notifications.update(cx, |n, cx| n.clear_timeline(cx));
            });
        let now = chrono::Local::now();
        let entries: Vec<AnyElement> = self
            .notifications
            .read(cx)
            .timeline()
            .iter()
            .rev()
            .map(|e| render_entry(e, now, cx))
            .collect();
        div()
            .id("notifications-timeline")
            .debug_selector(|| "notifications-timeline".into())
            .flex()
            .flex_col()
            .flex_1()
            .min_h(px(0.0))
            .pt(px(GROUP_TOP))
            .gap(px(TITLE_GAP))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .pl(px(TITLE_LEFT))
                    .pr(px(LINK_RIGHT))
                    .child(div().text_size(small).text_color(muted).child(TIMELINE))
                    .child(clear),
            )
            .child(
                div()
                    .id("notifications-list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .pl(px(LIST_LEFT))
                    .pr(px(LIST_RIGHT))
                    .gap(px(LIST_GAP))
                    .children(entries),
            )
            .into_any_element()
    }
}

/// A notification of the timeline (`NotificationComponent`): the icon,
/// then the title (bold when there is content) with the time at its
/// right, the content and the action links.
fn render_entry(e: &TimelineEntry, now: chrono::DateTime<chrono::Local>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let n = &e.notification;
    let id = e.id;
    let has_content = !n.content.is_empty();
    let time = div()
        .flex_none()
        .pr(px(TEXT_RIGHT))
        .text_size(theme.ui_size() - px(1.0))
        .text_color(theme.color(UiRole::TextMuted))
        .whitespace_nowrap()
        .child(e.time_text(now));
    // The time goes after the title, or after the content without one.
    let first = if !n.title.is_empty() {
        Some(
            div()
                .when(has_content, |d| d.font_weight(FontWeight::BOLD))
                .child(n.title.clone()),
        )
    } else if has_content {
        Some(div().child(n.content.clone()))
    } else {
        None
    };
    let mut text = div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(LINE_GAP));
    if let Some(first) = first {
        text = text.child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .pr(px(TEXT_RIGHT))
                        .child(first),
                )
                .child(time),
        );
    }
    if has_content && !n.title.is_empty() {
        text = text.child(div().pr(px(TEXT_RIGHT)).child(n.content.clone()));
    }
    if !n.actions.is_empty() {
        let links = n.actions.iter().enumerate().map(|(index, action)| {
            let command = action.command.clone();
            let args = action.args.clone();
            ui::link::link(
                ("notification-action", id as usize * 64 + index),
                action.text.clone(),
                cx,
            )
            .debug_selector(move || format!("notification-action-{id}-{index}"))
            .on_click(move |_, window, cx| {
                let (command, args) = (command.clone(), args.clone());
                // After this click: the command may reach the workbench.
                window.defer(cx, move |window, cx| {
                    if let Err(e) = ui::command::invoke(&command, args, window, cx) {
                        crate::env::marker(&format!("notification action {command}: {e:?}"));
                    }
                });
            })
            .into_any_element()
        });
        text = text.child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(ACTION_GAP))
                .children(links),
        );
    }
    div()
        .id(("notification-entry", id as usize))
        .debug_selector(move || format!("notification-entry-{id}"))
        .flex()
        .flex_row()
        .flex_none()
        .pt(px(ENTRY_INSET))
        .pl(px(ENTRY_INSET))
        .pb(px(ENTRY_INSET))
        .gap(px(ICON_GAP))
        .child(
            div()
                .flex_none()
                .child(Icon::new(n.kind.icon()).size(px(ICON))),
        )
        .child(text)
        .into_any_element()
}

impl Focusable for NotificationsToolWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ToolWindowView for NotificationsToolWindow {}

impl Render for NotificationsToolWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(cx).into_any_element();
        let body = if self.notifications.read(cx).timeline().is_empty() {
            self.render_empty(cx)
        } else {
            self.render_timeline(cx)
        };
        div()
            .id("notifications-tool-window-content")
            .key_context("NotificationsToolWindow")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(body)
    }
}
