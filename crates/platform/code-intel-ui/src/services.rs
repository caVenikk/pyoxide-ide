//! The Language Services widget of the status bar and its popup, as the
//! reference's `LanguageServiceWidget` with `LspClientWidgetItem` rows:
//! the icons of the servers that serve the current file (or `{}`), a
//! tooltip naming them, and a popup with the servers "Running on Current
//! File" and "Running on Other Files", their states and inline actions.

use std::cell::Cell;
use std::rc::Rc;

use code_intel::{ServerId, ServerState, ServerStatus};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Corner, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    WeakEntity, Window, anchored, canvas, deferred, div, point, prelude::FluentBuilder, px,
};
use name_match::{Matcher, MatchingMode};
use serde_json::{Value as Json, json};
use themes::{Metric, UiRole};
use ui::components::Tooltip;
use ui::pointer::PointerExt as _;
use ui::{Icon, IconName, ThemeExt};

use crate::view::CodeIntelView;

/// Icons in the status bar at most; with more servers, three and `+N`.
pub const MAX_ICONS: usize = 4;

pub const TITLE: &str = "Language Services";
pub const CURRENT_FILE: &str = "Running on Current File";
pub const OTHER_FILES: &str = "Running on Other Files";
pub const NO_SERVICES: &str = "No Services";
pub const RESTART: &str = "Restart Server";
pub const STOP: &str = "Stop and Autorun Server as Needed";
pub const ERROR_OUTPUT: &str = "Open Server Error Output";
pub const SETTINGS: &str = "Open Settings";

/// A server as the widget shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerItem {
    pub id: ServerId,
    /// The tool's name (`Ruff`), for the tooltip and the notices.
    pub title: String,
    /// The row's text: the server's name and version, then its state
    /// (`ruff 0.15.20 | Initializing…`).
    pub label: String,
    pub icon: IconName,
    pub status_bar_icon: IconName,
    /// It terminated: an error mark on its icons.
    pub error: bool,
    /// Its standard error, when it terminated with some.
    pub error_output: Option<String>,
    /// It serves the current file.
    pub current: bool,
    pub state: ServerState,
    /// Version, for the tooltip.
    pub version: Option<String>,
}

impl ServerItem {
    pub fn new(
        status: &ServerStatus,
        title: String,
        icon: Option<IconName>,
        status_bar_icon: Option<IconName>,
        current: bool,
    ) -> Self {
        let base = status.label();
        let label = match &status.state {
            ServerState::Initializing => format!("{base} | Initializing…"),
            ServerState::Running => base,
            ServerState::Stopped => format!("{base} | Stopped"),
            ServerState::Terminated { .. } => format!("{base} | Terminated"),
        };
        let error_output = match &status.state {
            ServerState::Terminated { error_output } if !error_output.trim().is_empty() => {
                Some(error_output.clone())
            }
            _ => None,
        };
        Self {
            id: status.id.clone(),
            title,
            label,
            icon: icon.unwrap_or(IconName::JSON_OBJECT),
            status_bar_icon: status_bar_icon.or(icon).unwrap_or(IconName::JSON_OBJECT),
            error: matches!(status.state, ServerState::Terminated { .. }),
            error_output,
            current,
            state: status.state.clone(),
            version: status.version.clone(),
        }
    }

    /// The inline actions of its row (`LspClientWidgetItem.
    /// createWidgetInlineActions`): the error output of a terminated
    /// server, Restart for a server of the current file, Stop for another
    /// one that runs (Restart when it terminated), and the settings.
    pub fn actions(&self) -> Vec<InlineAction> {
        let mut actions = Vec::new();
        if self.error_output.is_some() {
            actions.push(InlineAction::ErrorOutput);
        }
        if self.current {
            actions.push(InlineAction::Restart);
        } else {
            match self.state {
                ServerState::Initializing | ServerState::Running => {
                    actions.push(InlineAction::Stop)
                }
                ServerState::Terminated { .. } => actions.push(InlineAction::Restart),
                ServerState::Stopped => {}
            }
        }
        actions.push(InlineAction::Settings);
        actions
    }
}

/// A button at the end of a server's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineAction {
    ErrorOutput,
    Restart,
    Stop,
    Settings,
}

impl InlineAction {
    pub fn title(self) -> &'static str {
        match self {
            InlineAction::ErrorOutput => ERROR_OUTPUT,
            InlineAction::Restart => RESTART,
            InlineAction::Stop => STOP,
            InlineAction::Settings => SETTINGS,
        }
    }

    fn icon(self) -> IconName {
        match self {
            InlineAction::ErrorOutput => IconName::ERROR,
            InlineAction::Restart | InlineAction::Stop => IconName::STOP_AND_RESTART,
            InlineAction::Settings => IconName::SETTINGS_SMALL,
        }
    }

    fn key(self) -> &'static str {
        match self {
            InlineAction::ErrorOutput => "error-output",
            InlineAction::Restart => "restart",
            InlineAction::Stop => "stop",
            InlineAction::Settings => "settings",
        }
    }
}

/// What the widget shows (`LanguageServiceWidget.getWidgetState`); the
/// widget is hidden without servers.
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetState {
    /// Icons of the servers of the current file, errors first; `{}` when
    /// none serves it.
    pub icons: Vec<(IconName, bool)>,
    /// `+N` when there are more servers than icons.
    pub text: Option<String>,
    pub tooltip: String,
}

impl WidgetState {
    pub fn new(items: &[ServerItem]) -> Option<Self> {
        if items.is_empty() {
            return None;
        }
        let current: Vec<&ServerItem> = items.iter().filter(|i| i.current).collect();
        if current.is_empty() {
            return Some(Self {
                icons: vec![(IconName::JSON_OBJECT, false)],
                text: None,
                tooltip: TITLE.to_owned(),
            });
        }
        let shown = if current.len() > MAX_ICONS {
            MAX_ICONS - 1
        } else {
            current.len()
        };
        let lines: Vec<String> = current
            .iter()
            .map(|i| match &i.version {
                Some(v) => format!("- {} {v}", i.title),
                None => format!("- {}", i.title),
            })
            .collect();
        Some(Self {
            icons: current[..shown]
                .iter()
                .map(|i| (i.status_bar_icon, i.error))
                .collect(),
            text: (current.len() > MAX_ICONS).then(|| format!("+{}", current.len() - shown)),
            tooltip: format!(
                "Language Services running on the current file:\n{}",
                lines.join("\n")
            ),
        })
    }

    pub fn state_json(&self) -> Json {
        json!({
            "icons": self.icons.iter().map(|(icon, error)| json!({
                "icon": icon.0,
                "error": error,
            })).collect::<Vec<_>>(),
            "text": self.text,
            "tooltip": self.tooltip,
        })
    }
}

/// The widget in the status bar; a click opens the popup.
pub fn status_widget(view: &Entity<CodeIntelView>, cx: &App) -> Option<AnyElement> {
    let v = view.read(cx);
    let state = WidgetState::new(&v.server_items(cx))?;
    let bounds = v.widget_bounds();
    let theme = cx.theme();
    let hover = theme.color(UiRole::Hover);
    let open = view.downgrade();
    let mut icons = Vec::new();
    for (i, (icon, error)) in state.icons.iter().enumerate() {
        if i > 0 {
            icons.push(div().w(px(3.0)).into_any_element());
        }
        icons.push(marked_icon(*icon, *error).into_any_element());
    }
    Some(
        div()
            .id("status-language-services")
            .debug_selector(|| "status-language-services".into())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.0))
            .h(px(22.0))
            .px(px(6.0))
            .rounded(px(4.0))
            .whitespace_nowrap()
            .hover(move |s| s.bg(hover))
            .children(icons)
            .children(state.text.clone())
            .tooltip(Tooltip::text(state.tooltip.clone(), None))
            .on_click(move |_, window, cx| {
                if let Some(view) = open.upgrade() {
                    view.update(cx, |v, cx| v.toggle_services_popup(window, cx));
                }
            })
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .into_any_element(),
    )
}

/// An icon with the error mark over it.
fn marked_icon(icon: IconName, error: bool) -> impl IntoElement {
    div()
        .relative()
        .flex_none()
        .size(px(16.0))
        .child(Icon::new(icon).size(px(16.0)))
        .when(error, |d| {
            d.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .child(Icon::new(IconName::ERROR_MARK).size(px(16.0))),
            )
        })
}

/// A row of the popup.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    Header(&'static str),
    NoServices,
    Server(ServerItem),
}

/// The popup of the widget: it takes the focus (keys of a list popup,
/// speed search) and closes on Escape, a click outside or when the focus
/// leaves it.
pub struct ServicesPopup {
    owner: WeakEntity<CodeIntelView>,
    focus: FocusHandle,
    rows: Vec<Row>,
    selected: Option<usize>,
    filter: String,
    dismissed: bool,
    /// Had the focus before; gets it back when the popup closes.
    origin: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DismissEvent> for ServicesPopup {}

impl Focusable for ServicesPopup {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl ServicesPopup {
    pub fn open(
        owner: WeakEntity<CodeIntelView>,
        items: Vec<ServerItem>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let origin = window.focused(cx);
        let popup = cx.new(|cx: &mut Context<Self>| {
            let focus = cx.focus_handle();
            let subscriptions = vec![
                cx.on_blur(&focus, window, |this, window, cx| this.dismiss(window, cx)),
                cx.observe_window_activation(window, |this, window, cx| {
                    if !window.is_window_active() {
                        this.dismiss(window, cx);
                    }
                }),
            ];
            let mut popup = Self {
                owner,
                focus,
                rows: Vec::new(),
                selected: None,
                filter: String::new(),
                dismissed: false,
                origin,
                _subscriptions: subscriptions,
            };
            popup.set_items(items);
            popup
        });
        let focus = popup.read(cx).focus.clone();
        window.focus(&focus);
        popup
    }

    /// The servers changed while the popup is open.
    pub fn set_items(&mut self, items: Vec<ServerItem>) {
        let selected = self.selected_item().map(|i| i.id.clone());
        let (current, other): (Vec<_>, Vec<_>) = items.into_iter().partition(|i| i.current);
        let mut rows = vec![Row::Header(CURRENT_FILE)];
        if current.is_empty() {
            rows.push(Row::NoServices);
        }
        rows.extend(current.into_iter().map(Row::Server));
        // The section of other files is not shown without servers there.
        if !other.is_empty() {
            rows.push(Row::Header(OTHER_FILES));
            rows.extend(other.into_iter().map(Row::Server));
        }
        self.rows = rows;
        self.selected = selected
            .and_then(|id| {
                self.rows
                    .iter()
                    .position(|r| matches!(r, Row::Server(i) if i.id == id))
            })
            .or_else(|| self.selectable().first().copied());
    }

    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    /// Closes the popup and gives the focus back.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dismissed {
            self.dismissed = true;
            if self.focus.contains_focused(window, cx)
                && let Some(origin) = &self.origin
            {
                window.focus(origin);
            }
            cx.emit(DismissEvent);
            cx.notify();
        }
    }

    fn matcher(&self) -> Option<Matcher> {
        (!self.filter.is_empty())
            .then(|| Matcher::new(&format!("*{}", self.filter), MatchingMode::IgnoreCase))
    }

    /// Rows shown with the speed search's filter: server rows that match,
    /// and the headers and placeholder when nothing is filtered.
    fn shown(&self) -> Vec<usize> {
        let matcher = self.matcher();
        (0..self.rows.len())
            .filter(|&i| match (&self.rows[i], &matcher) {
                (_, None) => true,
                (Row::Server(item), Some(m)) => m.matches(&item.label),
                _ => false,
            })
            .collect()
    }

    fn selectable(&self) -> Vec<usize> {
        self.shown()
            .into_iter()
            .filter(|&i| matches!(self.rows[i], Row::Server(_)))
            .collect()
    }

    fn selected_item(&self) -> Option<&ServerItem> {
        match self.rows.get(self.selected?)? {
            Row::Server(item) => Some(item),
            _ => None,
        }
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.selectable();
        if rows.is_empty() {
            return;
        }
        let at = self
            .selected
            .and_then(|s| rows.iter().position(|r| *r == s))
            .unwrap_or(0);
        let next = crate::completion::moved(at, rows.len(), delta, true);
        self.selected = Some(rows[next]);
        cx.notify();
    }

    fn select_next(&mut self, _: &ui::menu::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn select_previous(
        &mut self,
        _: &ui::menu::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(-1, cx);
    }

    /// Enter on a row: the server's settings (the row's own action).
    fn confirm(&mut self, _: &ui::menu::Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.selected_item().cloned() {
            self.run(&item, InlineAction::Settings, window, cx);
        }
    }

    fn cancel(&mut self, _: &ui::menu::Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.filter.is_empty() {
            self.dismiss(window, cx);
        } else {
            self.filter.clear();
            self.fix_selection();
            cx.notify();
        }
    }

    fn fix_selection(&mut self) {
        let rows = self.selectable();
        if self.selected.is_none_or(|s| !rows.contains(&s)) {
            self.selected = rows.first().copied();
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        if k.modifiers.control || k.modifiers.platform || k.modifiers.alt || k.modifiers.function {
            return;
        }
        if k.key == "backspace" {
            if self.filter.pop().is_some() {
                self.fix_selection();
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        let Some(text) = k.key_char.as_deref() else {
            return;
        };
        let before = self.filter.len();
        for c in text.chars() {
            if c.is_alphanumeric() || (!self.filter.is_empty() && " .-_".contains(c)) {
                self.filter.push(c);
            }
        }
        if self.filter.len() != before {
            self.fix_selection();
            cx.stop_propagation();
            cx.notify();
        }
    }

    /// Runs an action of a server's row; the popup closes.
    fn run(
        &mut self,
        item: &ServerItem,
        action: InlineAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss(window, cx);
        let item = item.clone();
        if let Some(owner) = self.owner.upgrade() {
            owner.update(cx, |v, cx| v.run_server_action(&item, action, window, cx));
        }
    }

    /// The popup's state for `code.state`.
    pub fn state_json(&self) -> Json {
        let shown = self.shown();
        json!({
            "title": TITLE,
            "filter": self.filter,
            "selected": self.selected_item().map(|i| i.label.clone()),
            "rows": shown.iter().map(|&i| match &self.rows[i] {
                Row::Header(text) => json!({ "header": text }),
                Row::NoServices => json!({ "label": NO_SERVICES, "enabled": false }),
                Row::Server(item) => json!({
                    "server": item.id,
                    "label": item.label,
                    "error": item.error,
                    "actions": item.actions().iter().map(|a| a.title()).collect::<Vec<_>>(),
                }),
            }).collect::<Vec<_>>(),
        })
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row_height = theme.metric(Metric::RowHeight);
        let muted = theme.color(UiRole::TextMuted);
        match &self.rows[index] {
            Row::Header(text) => div()
                .flex()
                .items_center()
                .h(row_height)
                .px(px(16.0))
                .text_color(muted)
                .whitespace_nowrap()
                .child(*text)
                .into_any_element(),
            Row::NoServices => div()
                .flex()
                .items_center()
                .h(row_height)
                .px(px(16.0))
                .text_color(theme.color(UiRole::TextDisabled))
                .whitespace_nowrap()
                .child(NO_SERVICES)
                .into_any_element(),
            Row::Server(item) => {
                let selected = self.selected == Some(index);
                let selection = theme.color(UiRole::Selection);
                let actions: Vec<AnyElement> = item
                    .actions()
                    .into_iter()
                    .map(|action| {
                        let item = item.clone();
                        ui::components::small_icon_button(
                            SharedString::from(format!("{}-{}", item.id, action.key())),
                            action.icon(),
                            true,
                            cx,
                        )
                        .tooltip(Tooltip::text(action.title(), None))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.run(&item, action, window, cx);
                        }))
                        .into_any_element()
                    })
                    .collect();
                let clicked = item.clone();
                div()
                    .id(("language-service", index))
                    .debug_selector({
                        let label = item.label.clone();
                        move || format!("language-service:{label}")
                    })
                    .h(row_height)
                    .px(px(8.0))
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(8.0))
                            .rounded(px(4.0))
                            .when(selected, |d| d.bg(selection))
                            .child(marked_icon(item.icon, item.error))
                            .child(
                                div()
                                    .flex_1()
                                    .pl(px(2.0))
                                    .pr(px(16.0))
                                    .whitespace_nowrap()
                                    .child(item.label.clone()),
                            )
                            .children(actions),
                    )
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.selected != Some(index) {
                            this.selected = Some(index);
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run(&clicked, InlineAction::Settings, window, cx);
                    }))
                    .into_any_element()
            }
        }
    }
}

impl Render for ServicesPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .shown()
            .into_iter()
            .map(|i| self.render_row(i, cx))
            .collect();
        let theme = cx.theme();
        let text = theme.color(UiRole::Text);
        div()
            .id("language-services-popup")
            .debug_selector(|| "language-services-popup".into())
            .key_context(ui::menu::KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_key_down(cx.listener(Self::key_down))
            .occlude()
            .flex()
            .flex_col()
            .min_w(px(300.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(theme.color(UiRole::PopupBorder))
            .bg(theme.color(UiRole::SurfaceRaised))
            .shadow(ui::components::popup_shadow())
            .font_family(theme.ui_family())
            .text_size(theme.ui_size())
            .text_color(text)
            .child(
                div()
                    .flex()
                    .justify_center()
                    .pt(px(12.0))
                    .pb(px(10.0))
                    .px(px(10.0))
                    .font_weight(FontWeight::BOLD)
                    .whitespace_nowrap()
                    .child(TITLE),
            )
            .when(!self.filter.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(4.0))
                        .mx(px(12.0))
                        .mb(px(4.0))
                        .pb(px(8.0))
                        .pl(px(8.0))
                        .border_b_1()
                        .border_color(theme.color(UiRole::PopupBorder))
                        .child(Icon::new(IconName::SEARCH))
                        .child(self.filter.clone()),
                )
            })
            // The hand over the list (`ListPopupImpl`).
            .child(
                div()
                    .flex()
                    .flex_col()
                    .pb(px(8.0))
                    .children(rows)
                    .pointing_hand(),
            )
    }
}

/// Draws the open popup with its bottom left corner at the widget's top
/// left (as status bar popups open), over a backdrop that closes it on a
/// click.
pub fn overlay(popup: &Entity<ServicesPopup>, at: Bounds<Pixels>, window: &Window) -> AnyElement {
    let size = window.viewport_size();
    let close = popup.downgrade();
    deferred(
        anchored().position(point(px(0.0), px(0.0))).child(
            div()
                .id("language-services-backdrop")
                .occlude()
                .w(size.width)
                .h(size.height)
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    if let Some(popup) = close.upgrade() {
                        popup.update(cx, |p, cx| p.dismiss(window, cx));
                    }
                })
                .child(
                    anchored()
                        .position(at.origin)
                        .anchor(Corner::BottomLeft)
                        .snap_to_window_with_margin(px(4.0))
                        .child(popup.clone()),
                ),
        ),
    )
    .with_priority(1)
    .into_any_element()
}

/// Where the widget was drawn last.
pub type WidgetBounds = Rc<Cell<Bounds<Pixels>>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn status(id: &str, version: Option<&str>, state: ServerState) -> ServerStatus {
        let mut s = ServerStatus::new(ServerId::new(id), id);
        s.version = version.map(Into::into);
        s.state = state;
        s
    }

    fn item(id: &str, state: ServerState, current: bool) -> ServerItem {
        let title = id.to_uppercase();
        ServerItem::new(&status(id, Some("1.0"), state), title, None, None, current)
    }

    #[test]
    fn rows_read_like_the_reference() {
        let running = item("ruff", ServerState::Running, true);
        assert_eq!(running.label, "ruff 1.0");
        assert_eq!(
            running.actions(),
            [InlineAction::Restart, InlineAction::Settings]
        );
        let starting = item("ty", ServerState::Initializing, false);
        assert_eq!(starting.label, "ty 1.0 | Initializing…");
        assert_eq!(
            starting.actions(),
            [InlineAction::Stop, InlineAction::Settings]
        );
        let stopped = item("ty", ServerState::Stopped, false);
        assert_eq!(stopped.label, "ty 1.0 | Stopped");
        assert_eq!(stopped.actions(), [InlineAction::Settings]);
        let crashed = item(
            "ty",
            ServerState::Terminated {
                error_output: "panic".into(),
            },
            false,
        );
        assert_eq!(crashed.label, "ty 1.0 | Terminated");
        assert!(crashed.error);
        assert_eq!(
            crashed.actions(),
            [
                InlineAction::ErrorOutput,
                InlineAction::Restart,
                InlineAction::Settings
            ]
        );
    }

    #[test]
    fn the_widget_shows_the_servers_of_the_current_file() {
        assert_eq!(WidgetState::new(&[]), None);
        let other = item("ruff", ServerState::Running, false);
        let state = WidgetState::new(std::slice::from_ref(&other)).unwrap();
        assert_eq!(state.icons, [(IconName::JSON_OBJECT, false)]);
        assert_eq!(state.tooltip, TITLE);
        let current = item("ruff", ServerState::Running, true);
        let state = WidgetState::new(std::slice::from_ref(&current)).unwrap();
        assert_eq!(
            state.tooltip,
            "Language Services running on the current file:\n- RUFF 1.0"
        );
        // More than four: three icons and `+N`.
        let five: Vec<ServerItem> = (0..5)
            .map(|i| item(&format!("s{i}"), ServerState::Running, true))
            .collect();
        let state = WidgetState::new(&five).unwrap();
        assert_eq!(state.icons.len(), 3);
        assert_eq!(state.text.as_deref(), Some("+2"));
        let four = &five[..4];
        assert_eq!(WidgetState::new(four).unwrap().icons.len(), 4);
    }
}
