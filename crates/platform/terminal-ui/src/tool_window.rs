//! The Terminal tool window (the reference's reworked terminal,
//! `TerminalToolWindowManager`, `TerminalTabsManager`): a header with the
//! title, one tab per session (its name and ×, the selected tab outlined,
//! "+" and the chevron after them; ⋮ and "−" on the right) and the selected
//! session below.
//!
//! The window does not start sessions itself: "+" and New Tab ask the owner
//! ([`TerminalToolWindowEvent::NewTab`]), which plans the session with the
//! product's customizers and hands it over with [`TerminalToolWindow::add_tab`].

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px,
};
use serde_json::{Value, json};
use themes::{Metric, UiRole};
use ui::components::{Tooltip, icon_button};
use ui::icons::{Icon, IconName};
use ui::theme::ThemeExt;

use crate::commands::{CloseTab, NewTab, NextTab, PreviousTab, RenameTab};
use crate::{SessionView, SessionViewEvent};

/// The key context of the tool window (its header and sessions).
pub const KEY_CONTEXT: &str = "TerminalToolWindow";
/// The default name of a tab (`TerminalOptionsProvider.tabName`).
pub const DEFAULT_TAB_NAME: &str = "Local";

const BUTTON: f32 = 24.0;
const ICON: f32 = 16.0;

/// What the tool window asks of its owner.
#[derive(Clone, Debug, PartialEq)]
pub enum TerminalToolWindowEvent {
    /// Start a new session in a new tab ("+", New Tab).
    NewTab,
    /// Hide the tool window ("−").
    Hide,
    /// The last tab was closed: the tool window hides.
    Emptied,
    /// A session has a new copy of its screen: draw the window now.
    Redraw,
}

/// One tab: a session and its name.
pub struct TerminalTab {
    id: u64,
    name: SharedString,
    session: Entity<SessionView>,
    _subscription: Subscription,
}

impl TerminalTab {
    pub fn name(&self) -> &SharedString {
        &self.name
    }

    pub fn session(&self) -> &Entity<SessionView> {
        &self.session
    }
}

/// The Terminal tool window.
pub struct TerminalToolWindow {
    tabs: Vec<TerminalTab>,
    selected: Option<usize>,
    focus: FocusHandle,
    next_id: u64,
    /// "Close session when it ends" (`TerminalOptionsProvider.closeSessionOnLogout`).
    close_on_exit: bool,
}

impl EventEmitter<TerminalToolWindowEvent> for TerminalToolWindow {}

impl Focusable for TerminalToolWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// A name for a new tab that no tab has: `base`, `base (2)`, `base (3)`, ...
/// (`UniqueNameGenerator.generateUniqueName` as `createDefaultTabName`
/// calls it).
pub fn unique_name<'a>(base: &str, taken: impl IntoIterator<Item = &'a str> + Clone) -> String {
    let free = |name: &str| !taken.clone().into_iter().any(|t| t == name);
    if free(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} ({i})"))
        .find(|name| free(name))
        .expect("an unused name")
}

impl TerminalToolWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tabs: Vec::new(),
            selected: None,
            focus: cx.focus_handle(),
            next_id: 0,
            close_on_exit: true,
        }
    }

    pub fn tabs(&self) -> &[TerminalTab] {
        &self.tabs
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The session of the selected tab.
    pub fn selected_session(&self) -> Option<&Entity<SessionView>> {
        self.selected
            .and_then(|i| self.tabs.get(i))
            .map(|t| &t.session)
    }

    /// Where activating the tool window moves the focus: the selected
    /// session, or the tool window itself while it has no tab.
    pub fn content_focus(&self, cx: &App) -> FocusHandle {
        match self.selected_session() {
            Some(session) => session.focus_handle(cx),
            None => self.focus.clone(),
        }
    }

    pub fn set_close_on_exit(&mut self, close: bool) {
        self.close_on_exit = close;
    }

    /// A name for a new tab.
    pub fn new_tab_name(&self) -> String {
        unique_name(DEFAULT_TAB_NAME, self.tabs.iter().map(|t| t.name.as_ref()))
    }

    /// Adds a tab for `session` after the selected one and selects it; the
    /// focus moves into it when the tool window had it.
    pub fn add_tab(
        &mut self,
        session: Entity<SessionView>,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let name = name.unwrap_or_else(|| self.new_tab_name()).into();
        let subscription =
            cx.subscribe_in(
                &session,
                window,
                move |this, _, event, window, cx| match event {
                    SessionViewEvent::Redraw => cx.emit(TerminalToolWindowEvent::Redraw),
                    SessionViewEvent::Exited(_) if this.close_on_exit => {
                        if let Some(index) = this.tabs.iter().position(|t| t.id == id) {
                            this.close(index, window, cx);
                        }
                    }
                    _ => {}
                },
            );
        let index = self.selected.map_or(self.tabs.len(), |s| s + 1);
        self.tabs.insert(
            index,
            TerminalTab {
                id,
                name,
                session: session.clone(),
                _subscription: subscription,
            },
        );
        let had_focus = self.focus.contains_focused(window, cx);
        self.selected = Some(index);
        if had_focus {
            window.focus(&session.focus_handle(cx));
        }
        cx.notify();
        index
    }

    /// Closes a tab; its session ends with it. The focus, when it was in
    /// the tool window, goes to the tab selected next; the last tab closed
    /// hides the tool window.
    pub fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let had_focus = self.focus.contains_focused(window, cx);
        self.tabs.remove(index);
        self.selected = match self.selected {
            _ if self.tabs.is_empty() => None,
            Some(s) if s > index => Some(s - 1),
            Some(s) if s == index => Some(index.min(self.tabs.len() - 1)),
            other => other,
        };
        if had_focus {
            window.focus(&self.content_focus(cx));
        }
        if self.tabs.is_empty() {
            cx.emit(TerminalToolWindowEvent::Emptied);
        }
        cx.notify();
    }

    /// Selects a tab; the focus follows when it was in the tool window.
    pub fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        let had_focus = self.focus.contains_focused(window, cx);
        self.selected = Some(index);
        if had_focus {
            window.focus(&self.tabs[index].session.focus_handle(cx));
        }
        cx.notify();
    }

    /// Selects the next tab (`delta` 1) or the previous one (-1), around
    /// the ends.
    pub fn select_relative(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.tabs.len() as isize;
        if let Some(s) = self.selected
            && n > 1
        {
            self.select((s as isize + delta).rem_euclid(n) as usize, window, cx);
        }
    }

    /// Renames a tab; an empty name is refused.
    pub fn rename(&mut self, index: usize, name: &str, cx: &mut Context<Self>) -> bool {
        let name = name.trim();
        match self.tabs.get_mut(index) {
            Some(tab) if !name.is_empty() => {
                tab.name = name.to_owned().into();
                cx.notify();
                true
            }
            _ => false,
        }
    }

    /// The tabs and the selected session's state, for `terminal.state`.
    pub fn state_json(&self, cx: &App) -> Value {
        let mut state = match self.selected_session() {
            Some(session) => session.read(cx).state(),
            None => json!({}),
        };
        state["tabs"] = self
            .tabs
            .iter()
            .map(|t| {
                let s = t.session.read(cx);
                json!({
                    "name": t.name.as_ref(),
                    "title": s.title(),
                    "working_directory": s.working_directory(),
                    "exited": s.exit_status().is_some(),
                })
            })
            .collect();
        state["selected"] = json!(self.selected);
        state
    }

    // ---- commands -------------------------------------------------------

    fn new_tab(&mut self, _: &NewTab, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TerminalToolWindowEvent::NewTab);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = self.selected {
            self.close(s, window, cx);
        }
    }

    fn rename_tab(&mut self, action: &RenameTab, _: &mut Window, cx: &mut Context<Self>) {
        if let (Some(s), Some(name)) = (self.selected, &action.name) {
            self.rename(s, name, cx);
        }
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.select_relative(1, window, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        self.select_relative(-1, window, cx);
    }

    // ---- drawing --------------------------------------------------------

    fn render_tab(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[index];
        let theme = cx.theme();
        let selected = self.selected == Some(index);
        let hover = theme.color(UiRole::TabHover);
        let text = if selected {
            theme.color(UiRole::TabActiveText)
        } else {
            theme.color(UiRole::TabInactiveText)
        };
        let close = div()
            .id(("terminal-tab-close", index))
            .debug_selector(move || format!("terminal-tab-close-{index}"))
            .flex_none()
            .size(px(16.0))
            .rounded(px(3.0))
            .hover(move |s| s.bg(hover))
            .child(Icon::new(IconName::CLOSE_SMALL))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.close(index, window, cx);
            }));
        div()
            .id(("terminal-tab", index))
            .debug_selector(move || format!("terminal-tab-{index}"))
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .h(px(26.0))
            .pl(px(8.0))
            .pr(px(4.0))
            .gap(px(4.0))
            .rounded(theme.metric(Metric::ButtonRadius))
            .border_1()
            .when(selected, |d| {
                d.bg(theme.color(UiRole::TabActive))
                    .border_color(theme.color(UiRole::TabUnderline))
            })
            .when(!selected, |d| {
                d.border_color(gpui::transparent_black())
                    .hover(move |s| s.bg(hover))
            })
            .child(
                div()
                    .whitespace_nowrap()
                    .text_color(text)
                    .child(tab.name.clone()),
            )
            .child(close)
            .tooltip(Tooltip::text(tab.name.clone(), None))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.select(index, window, cx);
                if let Some(session) = this.selected_session() {
                    window.focus(&session.focus_handle(cx));
                }
            }))
            .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<AnyElement> = (0..self.tabs.len())
            .map(|i| self.render_tab(i, cx))
            .collect();
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
                    .mr(px(16.0))
                    .child("Terminal"),
            )
            .child(
                div()
                    .id("terminal-tabs")
                    .debug_selector(|| "terminal-tabs".into())
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .min_w(px(0.0))
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(
                icon_button(
                    "terminal-new-tab",
                    IconName::ADD,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .debug_selector(|| "terminal-new-tab".into())
                .ml(px(2.0))
                .tooltip(Tooltip::text("New Tab", None))
                .on_click(
                    cx.listener(|_, _: &ClickEvent, _, cx| {
                        cx.emit(TerminalToolWindowEvent::NewTab)
                    }),
                ),
            )
            .child(
                icon_button(
                    "terminal-new-session",
                    IconName::CHEVRON_DOWN,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .tooltip(Tooltip::text("New Predefined Session", None)),
            )
            .child(div().flex_1())
            .child(
                icon_button(
                    "terminal-options",
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
                    "terminal-hide",
                    IconName::HIDE,
                    px(BUTTON),
                    px(ICON),
                    false,
                    cx,
                )
                .debug_selector(|| "terminal-hide".into())
                .tooltip(Tooltip::text("Hide", None))
                .on_click(
                    cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(TerminalToolWindowEvent::Hide)),
                ),
            )
    }
}

impl Render for TerminalToolWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.render_header(cx).into_any_element();
        let body = self.selected_session().cloned();
        div()
            .id("terminal-tool-window-content")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::rename_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(
                div()
                    .id("terminal-session")
                    .debug_selector(|| "terminal-session".into())
                    .flex_1()
                    .min_h(px(0.0))
                    .children(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::unique_name;

    #[test]
    fn unique_names_take_the_first_free_number() {
        assert_eq!(unique_name("Local", []), "Local");
        assert_eq!(unique_name("Local", ["Local"]), "Local (2)");
        assert_eq!(
            unique_name("Local", ["Local", "Local (2)", "build"]),
            "Local (3)"
        );
        assert_eq!(unique_name("Local", ["Local (2)"]), "Local");
    }
}
